use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use futures::StreamExt;
use quick_xml::{Reader as XmlReader, Writer as XmlWriter, events::Event as XmlEvent};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::metrics::Metrics;
use crate::xml::{PushPort, PushPortEvent, PushPortUpdateKind};

mod api;
mod env;
mod helpers;
mod metrics;
mod redis;
mod stomp;
mod xml;

const STOMP_SUBSCRIPTION_COUNT: u64 = 2; // darwin.status + darwin.pushport-v16

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const SEQUENCE_MAX: u32 = 9_999_999;
const CONTENT_TYPE_HEADER: &str = "content_hyphen_type";

#[tokio::main]
async fn main() -> Result<()> {
	let env = env::load(None, cfg!(debug_assertions)).expect("Failed to load environment variables");

	let data_directory = Arc::new(PathBuf::from(&env.dpplf_data_directory));
	std::fs::create_dir_all(&*data_directory)
		.context(format!("Unable to create data directory '{}'", env.dpplf_data_directory))?;

	let metrics = Arc::new(Metrics::new());

	let redis = Arc::new(
		redis::Redis::connect(&env.redis_url, &env.redis_prefix, env.redis_ttl, metrics.clone())
			.context("Unable to connect to Redis")?,
	);
	println!("Connected to Redis server '{}'", &env.redis_url);

	let http_addr = format!("{}:{}", &env.dpplf_http_listen_address, env.dpplf_http_listen_port).parse().context(
		format!("Invalid HTTP_HOST/HTTP_PORT '{}:{}'", &env.dpplf_http_listen_address, env.dpplf_http_listen_port),
	)?;
	let api_state = api::AppState {
		redis: redis.clone(),
		metrics: metrics.clone(),
	};
	tokio::spawn(async move {
		if let Err(error) = api::serve(api_state, http_addr).await {
			eprintln!("HTTP API server failed: {error:#}");
		}
	});

	let host_id = helpers::anonymous_host_id();
	let unique_id = uuid::Uuid::new_v4();

	/******************************************************/

	let client_id = format!("{}-{}-{}", &env.dpplf_username, &host_id, &unique_id);

	let mut client = stomp::Client::connect(
		&format!("{}:{}", &env.dpplf_host, &env.dpplf_port),
		&env.dpplf_host,
		&env.dpplf_username,
		&env.dpplf_password,
		&client_id,
	)
	.await
	.context(format!("Unable to connect to server '{}:{}'", &env.dpplf_host, &env.dpplf_port,))?;

	println!("Connected to server '{}:{}' with client ID '{}'", &env.dpplf_host, &env.dpplf_port, &client_id);

	/******************************************************/

	let make_sub_headers =
		|topic_name: &str| vec![("activemq.subscriptionName".to_string(), format!("{}-{}", &host_id, topic_name))];

	let status_topic = "darwin.status";
	let mut status_subscription = client
		.subscribe(&format!("/topic/{}", status_topic), make_sub_headers(status_topic))
		.await
		.context(format!("Unable to subscribe to '{}' topic", status_topic))?;
	println!("Subscribed to '{}' topic", status_topic);

	let live_feed_topic = "darwin.pushport-v16";
	let mut live_feed_subscription = client
		.subscribe(&format!("/topic/{}", live_feed_topic), make_sub_headers(live_feed_topic))
		.await
		.context(format!("Unable to subscribe to '{}' topic", live_feed_topic))?;
	println!("Subscribed to '{}' topic", live_feed_topic);

	metrics.set_stomp_connected(true, STOMP_SUBSCRIPTION_COUNT);

	/******************************************************/

	let (channel_transmit, mut channel_receive) = tokio::sync::mpsc::channel::<(Vec<u8>, Option<String>)>(1024);

	let stomp_metrics = metrics.clone();

	tokio::spawn(async move {
		while let Some((body, content_type)) = channel_receive.recv().await {
			let data_directory = data_directory.clone();
			let database = redis.clone();
			let metrics = metrics.clone();
			match tokio::task::spawn_blocking(move || {
				process_frame_body(body, content_type, &data_directory, &database, &metrics)
			})
			.await
			{
				Ok(Ok(())) => {}
				Ok(Err(error)) => eprintln!("Failed to process STOMP frame: {error:#}"),
				Err(error) => eprintln!("Processing task panicked: {error}"),
			}
		}
	});

	/******************************************************/

	let mut last_sequence_number: Option<u32> = None;

	loop {
		tokio::select! {
			result = status_subscription.next() => {
				match result {
					Some(frame) => {
						if let Ok(text) = String::from_utf8(frame.body) {
							match text.trim() {
								"HBINT"         => eprintln!("Darwin: feed initialising timetable"),
								"HBFAIL"        => eprintln!("Darwin: feed going down"),
								"HBPENDING"     => eprintln!("Darwin: feed in failover mode"),
								"SHUTTING-DOWN" => eprintln!("Darwin: shutting down"),
								"SNAPSHOT"      => eprintln!("Darwin: snapshot in progress"),
								other           => eprintln!("Darwin status: '{other}'"),
							}
						}
					}

					None => {
						stomp_metrics.set_stomp_connected(false, 0);
						eprintln!("Status subscription stream ended");
					}
				}
			}

			result = live_feed_subscription.next() => {
				match result {
					Some(frame) => {
						stomp_metrics.record_stomp_message();

						/*
						eprintln!("============ FRAME ============");
						eprintln!("COMMAND: {}", frame.command);
						for (name, value) in &frame.headers {
							eprintln!("HEADER: {name}: {value}");
						}
						eprintln!("BODY: {} byte(s)", frame.body.len());
						eprintln!("===============================");
						*/

						if let Some(sequence_header) = frame.get_header("SequenceNumber") {
							if let Ok(sequence_number) = sequence_header.parse::<u32>() {
								if let Some(previous_sequence_number) = last_sequence_number {
									let expected_sequence_number = if previous_sequence_number == SEQUENCE_MAX { 0 } else { previous_sequence_number + 1 };

									if sequence_number != expected_sequence_number {
										eprintln!(
											"Sequence gap! Expected {expected_sequence_number}, got {sequence_number} (last: {previous_sequence_number}, missed: {})",
											sequence_number.saturating_sub(expected_sequence_number)
										);
									}
								}

								last_sequence_number = Some(sequence_number);
							}
						}

						let content_type = frame.get_header(CONTENT_TYPE_HEADER).map(str::to_string);
						if channel_transmit.send((frame.body, content_type)).await.is_err() {
							eprintln!("Channel closed unexpectedly");
							break;
						}
					}

					None => {
						stomp_metrics.set_stomp_connected(false, 0);
						eprintln!("Live feed subscription stream ended");
						break;
					}
				}
			}
		}
	}

	Ok(())
}

fn pretty_xml(xml: &str) -> String {
	let mut reader = XmlReader::from_str(xml);
	reader.config_mut().trim_text(true);

	let mut writer = XmlWriter::new_with_indent(Vec::new(), b'\t', 1);
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer) {
			Ok(XmlEvent::Eof) => break,
			Ok(event) => {
				if writer.write_event(event).is_err() {
					return xml.to_string();
				}
			}
			Err(_) => return xml.to_string(),
		}

		buffer.clear();
	}

	String::from_utf8(writer.into_inner()).unwrap_or_else(|_| xml.to_string())
}

fn process_frame_body(
	body: Vec<u8>,
	content_type: Option<String>,
	data_directory: &Path,
	database: &redis::Redis,
	metrics: &Metrics,
) -> Result<()> {
	let is_gzip = body.starts_with(&GZIP_MAGIC)
		|| content_type
			.as_deref()
			.is_some_and(|content_type| content_type.contains("gzip") || content_type.contains("octet-stream"));

	let xml = if is_gzip {
		let mut decoder = GzDecoder::new(body.as_slice());
		let mut decompressed = String::new();

		decoder.read_to_string(&mut decompressed)?;

		decompressed
	} else {
		String::from_utf8(body)?
	};

	let push_port = xml::parse(&xml)?;

	let filename = format!("{}.xml", push_port.timestamp.replace(':', "-"));
	std::fs::write(data_directory.join(filename), pretty_xml(&xml)).context("Writing XML to data directory")?;

	database.store(&push_port).context("Storing update in Redis")?;

	handle_push_port(push_port, metrics)
}

fn handle_push_port(push_port: PushPort, metrics: &Metrics) -> Result<()> {
	let kind = match push_port.update.kind {
		PushPortUpdateKind::Live => "LIVE",
		PushPortUpdateKind::Snapshot => "SNAPSHOT",
	};

	for event in push_port.update.events {
		metrics.record_event(&event);

		match event {
			PushPortEvent::TrainStatus(status) => {
				println!(
					"[{}]\t{}\tTRAIN STATUS\trid={} uid={} locations={}",
					push_port.timestamp,
					kind,
					status.darwin_timetable_id,
					status.schedule_uid,
					status.locations.len(),
				);
			}

			PushPortEvent::Schedule(schedule) => {
				println!(
					"[{}]\t{}\tSCHEDULE\trid={} uid={} toc={} locations={}",
					push_port.timestamp,
					kind,
					schedule.darwin_id,
					schedule.schedule_uid,
					schedule.operator_code,
					schedule.locations.len(),
				);
			}

			PushPortEvent::Deactivated {
				rid,
			} => {
				println!("[{}]\t{}\tDEACTIVATED\trid={}", push_port.timestamp, kind, rid);
			}

			PushPortEvent::Association(association) => {
				println!(
					"[{}]\t{}\tASSOCIATION\ttiploc={} category={:?}",
					push_port.timestamp, kind, association.tiploc, association.category,
				);
			}

			PushPortEvent::StationMessage(message) => {
				println!(
					"[{}]\t{}\tSTATION MESSAGE\tid={} severity={} stations={:?}",
					push_port.timestamp, kind, message.id, message.severity, message.stations,
				);
			}

			PushPortEvent::TrainAlert(alert) => {
				println!(
					"[{}]\t{}\tTRAIN ALERT\tid={} audience={:?}",
					push_port.timestamp, kind, alert.id, alert.audience,
				);
			}

			PushPortEvent::TrainOrder(order) => {
				println!(
					"[{}]\t{}\tTRAIN ORDER\ttiploc={} crs={} platform={}",
					push_port.timestamp, kind, order.tiploc, order.crs, order.platform,
				);
			}

			PushPortEvent::TrackingId(tracking) => {
				println!(
					"[{}]\t{}\tTRACKING ID\tcorrection={} area={} berth={}",
					push_port.timestamp, kind, tracking.correction, tracking.area, tracking.berth,
				);
			}

			PushPortEvent::Alarm(alarm) => {
				println!("[{}]\t{}\tALARM\taction={:?}", push_port.timestamp, kind, alarm.action,);
			}

			PushPortEvent::TimetableId(timetable) => {
				println!(
					"[{}]\t{}\tTIMETABLE ID\tid={} file={}",
					push_port.timestamp, kind, timetable.id, timetable.timetable_file,
				);
			}

			PushPortEvent::FormationLoading(loading) => {
				println!(
					"[{}]\t{}\tFORMATION LOADING\trid={} fid={} tiploc={} coaches={}",
					push_port.timestamp,
					kind,
					loading.darwin_id,
					loading.formation_id,
					loading.tiploc,
					loading.coaches.len(),
				);
			}

			PushPortEvent::ScheduleFormations(formations) => {
				println!(
					"[{}]\t{}\tSCHEDULE FORMATIONS\trid={} formations={}",
					push_port.timestamp,
					kind,
					formations.darwin_id,
					formations.formations.len(),
				);
			}
		}
	}

	Ok(())
}
