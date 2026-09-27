use anyhow::{Context, Result};
use redis::{Commands, Connection, Pipeline, SortedSetAddOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::metrics::Metrics;
use crate::xml::{
	PushPort, PushPortAssociation, PushPortEvent, PushPortForecastPlatformData, PushPortForecastTimeData,
	PushPortFormationLoading, PushPortSchedule, PushPortScheduleFormations, PushPortStationMessage,
	PushPortTimetableID, PushPortTrainStatus,
};

pub struct Redis {
	client: redis::Client,
	connection: Mutex<Connection>,
	metrics: Arc<Metrics>,

	key_prefix: String,
	stale_after_seconds: i64,
}

type Fields = Vec<(String, String)>;

impl Redis {
	pub fn connect(url: &str, key_prefix: &str, stale_after_seconds: u64, metrics: Arc<Metrics>) -> Result<Self> {
		let client = redis::Client::open(url).context("Invalid Redis URL")?;
		let connection = client.get_connection().context("Unable to connect to Redis")?;

		Ok(Self {
			client,
			connection: Mutex::new(connection),
			metrics,
			key_prefix: key_prefix.to_string(),
			stale_after_seconds: stale_after_seconds as i64,
		})
	}

	pub fn ping(&self) -> Option<f64> {
		let start = Instant::now();

		match self.with_connection(|connection| redis::cmd("PING").query::<String>(connection)) {
			Ok(_) => Some(start.elapsed().as_secs_f64() * 1000.0),
			Err(_) => None,
		}
	}

	pub fn key_count(&self) -> Option<u64> {
		self.with_connection(|connection| redis::cmd("DBSIZE").query::<u64>(connection)).ok()
	}

	pub fn hash(&self, key: &str) -> Result<HashMap<String, String>> {
		self.with_connection(|connection| connection.hgetall(key))
	}

	pub fn smembers(&self, key: &str) -> Result<Vec<String>> {
		self.with_connection(|connection| connection.smembers(key))
	}

	// Ascending order - matches ZADD score order (schedule sequence index, or first-seen time).
	pub fn zrange_all(&self, key: &str) -> Result<Vec<String>> {
		self.with_connection(|connection| connection.zrange(key, 0, -1))
	}

	// Descending order (most recently touched first) - unbounded, this is an internal API.
	pub fn zrevrange_all(&self, key: &str) -> Result<Vec<String>> {
		self.with_connection(|connection| connection.zrevrange(key, 0, -1))
	}

	pub fn store(&self, push_port: &PushPort) -> Result<()> {
		for event in &push_port.update.events {
			self.store_event(event)?;
		}

		Ok(())
	}

	fn store_event(&self, event: &PushPortEvent) -> Result<()> {
		match event {
			PushPortEvent::TrainStatus(status) => self.store_train_status(status),
			PushPortEvent::Schedule(schedule) => self.store_schedule(schedule),
			PushPortEvent::Deactivated {
				rid,
			} => self.store_deactivated(rid),
			PushPortEvent::Association(association) => self.store_association(association),
			PushPortEvent::StationMessage(message) => self.store_station_message(message),
			PushPortEvent::TimetableId(timetable) => self.store_timetable_id(timetable),
			PushPortEvent::FormationLoading(loading) => self.store_formation_loading(loading),
			PushPortEvent::ScheduleFormations(formations) => self.store_schedule_formations(formations),

			// Too rare in practice to justify bespoke mutable state - kept as a capped audit trail instead.
			PushPortEvent::TrainAlert(_)
			| PushPortEvent::TrainOrder(_)
			| PushPortEvent::TrackingId(_)
			| PushPortEvent::Alarm(_) => self.store_operational_event(event),
		}
	}

	fn store_train_status(&self, status: &PushPortTrainStatus) -> Result<()> {
		let now = now_epoch();
		let ttl = self.stale_after_seconds;
		let journey_key = self.key(&["journey", &status.darwin_timetable_id]);
		let location_order_key = self.key(&["journey", &status.darwin_timetable_id, "location_order"]);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field(&mut fields, "schedule_uid", status.schedule_uid.clone());
			field(&mut fields, "schedule_start_date", status.schedule_start_date.clone());
			field_bool(&mut fields, "is_reverse_formation", status.is_reverse_formation);
			field_num(&mut fields, "last_updated_at", now);

			if let Some(reason) = &status.delay_reason_code {
				field_num(&mut fields, "delay_reason_code", reason.code);
			}

			pipeline.hset_multiple(&journey_key, &fields).ignore();
			pipeline.expire(&journey_key, ttl).ignore();

			self.touch_indices(pipeline, &status.darwin_timetable_id, now, ttl, Some(&status.schedule_uid), None, None);

			for location in &status.locations {
				let location_id = location_id(
					&location.tiploc,
					location.working_arrival_time.as_deref(),
					location.working_departure_time.as_deref(),
					location.pass_time.as_deref(),
				);
				let location_key = self.key(&["journey", &status.darwin_timetable_id, "location", &location_id]);

				pipeline
					.zadd_options(&location_order_key, &location_id, now, &SortedSetAddOptions::add_only())
					.ignore();

				let mut location_fields: Fields = Vec::new();
				field_opt(&mut location_fields, "public_arrival_time", &location.public_arrival_time);
				field_opt(&mut location_fields, "public_departure_time", &location.public_departure_time);

				if let Some(arrival) = &location.arrive {
					push_time_data(&mut location_fields, "arrival", arrival);
				}
				if let Some(departure) = &location.depart {
					push_time_data(&mut location_fields, "departure", departure);
				}
				if let Some(pass) = &location.pass {
					push_time_data(&mut location_fields, "pass", pass);
				}
				if let Some(platform) = &location.platform {
					push_platform_data(&mut location_fields, platform);
				}
				if let Some(length) = location.length {
					field_num(&mut location_fields, "length_in_coaches", length);
				}

				field_bool(&mut location_fields, "is_suppressed", location.suppress);
				field_bool(&mut location_fields, "detach_front", location.detach_front);

				if !location_fields.is_empty() {
					pipeline.hset_multiple(&location_key, &location_fields).ignore();
					pipeline.expire(&location_key, ttl).ignore();
				}

				self.touch_tiploc_index(pipeline, &location.tiploc, &status.darwin_timetable_id, now, ttl);
			}

			pipeline.expire(&location_order_key, ttl).ignore();
		})
	}

	fn store_schedule(&self, schedule: &PushPortSchedule) -> Result<()> {
		let now = now_epoch();
		let ttl = self.stale_after_seconds;
		let journey_key = self.key(&["journey", &schedule.darwin_id]);
		let location_order_key = self.key(&["journey", &schedule.darwin_id, "location_order"]);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field(&mut fields, "schedule_uid", schedule.schedule_uid.clone());
			field(&mut fields, "schedule_start_date", schedule.schedule_start_date.clone());
			field(&mut fields, "headcode", schedule.headcode.clone());
			field(&mut fields, "operator_code", schedule.operator_code.clone());
			field(&mut fields, "category", schedule.category.clone());
			field(&mut fields, "status", schedule.status.clone());
			field_bool(&mut fields, "is_active", schedule.is_active);
			field_bool(&mut fields, "is_passenger", schedule.is_passenger);
			field_bool(&mut fields, "is_charter", schedule.is_charter);
			field_bool(&mut fields, "is_deleted", schedule.is_deleted);
			field_num(&mut fields, "last_updated_at", now);

			if let Some(reason) = &schedule.cancel_reason_code {
				field_num(&mut fields, "cancel_reason_code", reason.code);
			}

			pipeline.hset_multiple(&journey_key, &fields).ignore();
			pipeline.expire(&journey_key, ttl).ignore();

			self.touch_indices(
				pipeline,
				&schedule.darwin_id,
				now,
				ttl,
				Some(&schedule.schedule_uid),
				Some(&schedule.operator_code),
				Some(&schedule.headcode),
			);

			for (sequence_index, location) in schedule.locations.iter().enumerate() {
				let location_id = location_id(
					&location.tiploc,
					location.working_arrival_time.as_deref(),
					location.working_departure_time.as_deref(),
					location.pass_time.as_deref(),
				);

				pipeline.zadd(&location_order_key, &location_id, sequence_index as i64).ignore();

				let location_key = self.key(&["journey", &schedule.darwin_id, "location", &location_id]);
				let mut location_fields: Fields = Vec::new();
				field(&mut location_fields, "kind", format!("{:?}", location.kind));
				field_opt(&mut location_fields, "working_arrival_time", &location.working_arrival_time);
				field_opt(&mut location_fields, "working_departure_time", &location.working_departure_time);
				field_opt(&mut location_fields, "pass_time", &location.pass_time);
				field_opt(&mut location_fields, "public_arrival_time", &location.public_arrival_time);
				field_opt(&mut location_fields, "public_departure_time", &location.public_departure_time);
				field_opt(&mut location_fields, "platform", &location.platform);
				field_opt(&mut location_fields, "flags", &location.flags);
				field_opt(&mut location_fields, "original_flags", &location.original_flags);
				field_bool(&mut location_fields, "is_cancelled", location.is_cancelled);
				field_opt(&mut location_fields, "false_destination", &location.false_destination);

				if let Some(delay) = location.reroute_delay {
					field_num(&mut location_fields, "reroute_delay", delay);
				}

				pipeline.hset_multiple(&location_key, &location_fields).ignore();
				pipeline.expire(&location_key, ttl).ignore();

				self.touch_tiploc_index(pipeline, &location.tiploc, &schedule.darwin_id, now, ttl);
			}

			pipeline.expire(&location_order_key, ttl).ignore();
		})
	}

	fn store_deactivated(&self, rid: &str) -> Result<()> {
		let now = now_epoch();
		let ttl = self.stale_after_seconds;
		let journey_key = self.key(&["journey", rid]);
		let active_journeys_index = self.key(&["index", "active_journeys"]);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field_bool(&mut fields, "is_deactivated", true);
			field_num(&mut fields, "last_updated_at", now);

			pipeline.hset_multiple(&journey_key, &fields).ignore();
			pipeline.expire(&journey_key, ttl).ignore();

			pipeline.zrem(&active_journeys_index, rid).ignore();
		})
	}

	fn store_association(&self, association: &PushPortAssociation) -> Result<()> {
		let now = now_epoch();
		let ttl = self.stale_after_seconds;
		let association_key = self.key(&[
			"association",
			&association.tiploc,
			&association.main.darwin_id,
			&association.associated.darwin_id,
		]);
		let association_ref =
			format!("{}:{}:{}", association.tiploc, association.main.darwin_id, association.associated.darwin_id);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field(&mut fields, "category", format!("{:?}", association.category));
			field_bool(&mut fields, "is_cancelled", association.is_cancelled);
			field_bool(&mut fields, "is_deleted", association.is_deleted);
			field_opt(&mut fields, "main_working_arrival_time", &association.main.working_arrival_time);
			field_opt(&mut fields, "main_working_departure_time", &association.main.working_departure_time);
			field_opt(&mut fields, "main_public_arrival_time", &association.main.public_arrival_time);
			field_opt(&mut fields, "main_public_departure_time", &association.main.public_departure_time);
			field_opt(&mut fields, "main_pass_time", &association.main.pass_time);
			field_opt(&mut fields, "associated_working_arrival_time", &association.associated.working_arrival_time);
			field_opt(&mut fields, "associated_working_departure_time", &association.associated.working_departure_time);
			field_opt(&mut fields, "associated_public_arrival_time", &association.associated.public_arrival_time);
			field_opt(&mut fields, "associated_public_departure_time", &association.associated.public_departure_time);
			field_opt(&mut fields, "associated_pass_time", &association.associated.pass_time);

			pipeline.hset_multiple(&association_key, &fields).ignore();
			pipeline.expire(&association_key, ttl).ignore();

			for rid in [&association.main.darwin_id, &association.associated.darwin_id] {
				let associations_key = self.key(&["journey", rid, "associations"]);
				pipeline.sadd(&associations_key, &association_ref).ignore();
				pipeline.expire(&associations_key, ttl).ignore();

				self.touch_indices(pipeline, rid, now, ttl, None, None, None);
			}
		})
	}

	fn store_station_message(&self, message: &PushPortStationMessage) -> Result<()> {
		let ttl = self.stale_after_seconds;
		let message_id = message.id.to_string();
		let message_key = self.key(&["station_message", &message_id]);
		let stations_key = self.key(&["station_message", &message_id, "stations"]);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field(&mut fields, "category", format!("{:?}", message.category));
			field_num(&mut fields, "severity", message.severity);
			field_bool(&mut fields, "is_suppressed", message.suppress);
			field(&mut fields, "message_text", message.message.clone());

			pipeline.hset_multiple(&message_key, &fields).ignore();
			pipeline.expire(&message_key, ttl).ignore();

			if !message.stations.is_empty() {
				for crs in &message.stations {
					pipeline.sadd(&stations_key, crs).ignore();

					let station_index_key = self.key(&["index", "station_messages_by_station", crs]);
					pipeline.sadd(&station_index_key, &message_id).ignore();
					pipeline.expire(&station_index_key, ttl).ignore();
				}

				pipeline.expire(&stations_key, ttl).ignore();
			}
		})
	}

	fn store_timetable_id(&self, timetable: &PushPortTimetableID) -> Result<()> {
		let ttl = self.stale_after_seconds;
		let key = self.key(&["latest_timetable_reference"]);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field(&mut fields, "id", timetable.id.clone());
			field(&mut fields, "timetable_file", timetable.timetable_file.clone());
			field(&mut fields, "reference_file", timetable.reference_file.clone());
			field_num(&mut fields, "updated_at", now_epoch());

			pipeline.hset_multiple(&key, &fields).ignore();
			pipeline.expire(&key, ttl).ignore();
		})
	}

	fn store_formation_loading(&self, loading: &PushPortFormationLoading) -> Result<()> {
		let now = now_epoch();
		let ttl = self.stale_after_seconds;
		let location_id = location_id(
			&loading.tiploc,
			loading.working_arrival_time.as_deref(),
			loading.working_departure_time.as_deref(),
			loading.pass_time.as_deref(),
		);
		let loading_key = self.key(&["journey", &loading.darwin_id, "loading", &location_id]);
		let loading_locations_key = self.key(&["journey", &loading.darwin_id, "loading_locations"]);

		self.run(|pipeline| {
			let mut fields: Fields = Vec::new();
			field(&mut fields, "formation_id", loading.formation_id.clone());

			for coach in &loading.coaches {
				if let Some(percentage) = coach.percentage {
					field_num(&mut fields, &format!("coach_{}_percentage", coach.coach_number), percentage);
				}

				field_opt(&mut fields, &format!("coach_{}_source", coach.coach_number), &coach.source);
				field_opt(&mut fields, &format!("coach_{}_source_system", coach.coach_number), &coach.source_system);
			}

			pipeline.hset_multiple(&loading_key, &fields).ignore();
			pipeline.expire(&loading_key, ttl).ignore();

			// Index so all of a journey's loading snapshots can be listed without a key scan.
			pipeline.sadd(&loading_locations_key, &location_id).ignore();
			pipeline.expire(&loading_locations_key, ttl).ignore();

			self.touch_indices(pipeline, &loading.darwin_id, now, ttl, None, None, None);
		})
	}

	fn store_schedule_formations(&self, formations: &PushPortScheduleFormations) -> Result<()> {
		let now = now_epoch();
		let ttl = self.stale_after_seconds;
		let formations_key = self.key(&["journey", &formations.darwin_id, "formations"]);

		self.run(|pipeline| {
			for formation in &formations.formations {
				let formation_key = self.key(&["journey", &formations.darwin_id, "formation", &formation.formation_id]);

				let mut fields: Fields = Vec::new();
				for coach in &formation.coaches {
					field_opt(&mut fields, &format!("coach_{}_class", coach.coach_number), &coach.coach_class);
					field_opt(
						&mut fields,
						&format!("coach_{}_toilet_status", coach.coach_number),
						&coach.toilet_status,
					);
					field_opt(&mut fields, &format!("coach_{}_toilet_type", coach.coach_number), &coach.toilet_type);
				}

				if !fields.is_empty() {
					pipeline.hset_multiple(&formation_key, &fields).ignore();
					pipeline.expire(&formation_key, ttl).ignore();

					// Index so all of a journey's formations can be listed without a key scan.
					pipeline.sadd(&formations_key, &formation.formation_id).ignore();
					pipeline.expire(&formations_key, ttl).ignore();
				}
			}

			self.touch_indices(pipeline, &formations.darwin_id, now, ttl, None, None, None);
		})
	}

	fn store_operational_event(&self, event: &PushPortEvent) -> Result<()> {
		let ttl = self.stale_after_seconds;
		let stream_key = self.key(&["operational_events"]);
		let payload = format!("{:?}", event);

		self.run(|pipeline| {
			pipeline
				.cmd("XADD")
				.arg(&stream_key)
				.arg("MAXLEN")
				.arg("~")
				.arg(10_000)
				.arg("*")
				.arg("event")
				.arg(payload)
				.ignore();

			pipeline.expire(&stream_key, ttl).ignore();
		})
	}

	fn touch_indices(
		&self,
		pipeline: &mut Pipeline,
		rid: &str,
		now: i64,
		ttl: i64,
		schedule_uid: Option<&str>,
		operator_code: Option<&str>,
		headcode: Option<&str>,
	) {
		let active_journeys_index = self.key(&["index", "active_journeys"]);
		pipeline.zadd(&active_journeys_index, rid, now).ignore();
		pipeline.expire(&active_journeys_index, ttl).ignore();

		if let Some(schedule_uid) = schedule_uid.filter(|value| !value.is_empty()) {
			let schedule_uid_index = self.key(&["index", "journeys_by_schedule_uid", schedule_uid]);
			pipeline.sadd(&schedule_uid_index, rid).ignore();
			pipeline.expire(&schedule_uid_index, ttl).ignore();
		}

		if let Some(operator_code) = operator_code.filter(|value| !value.is_empty()) {
			let operator_index = self.key(&["index", "journeys_by_operator", operator_code]);
			pipeline.sadd(&operator_index, rid).ignore();
			pipeline.expire(&operator_index, ttl).ignore();
		}

		if let Some(headcode) = headcode.filter(|value| !value.is_empty()) {
			let headcode_index = self.key(&["index", "journeys_by_headcode", headcode]);
			pipeline.sadd(&headcode_index, rid).ignore();
			pipeline.expire(&headcode_index, ttl).ignore();
		}
	}

	fn touch_tiploc_index(&self, pipeline: &mut Pipeline, tiploc: &str, rid: &str, now: i64, ttl: i64) {
		let tiploc_index = self.key(&["index", "journeys_by_tiploc", tiploc]);
		pipeline.zadd(&tiploc_index, rid, now).ignore();
		pipeline.expire(&tiploc_index, ttl).ignore();
	}

	pub(crate) fn key(&self, parts: &[&str]) -> String {
		let mut key = self.key_prefix.clone();

		for part in parts {
			key.push(':');
			key.push_str(part);
		}

		key
	}

	fn run(&self, build: impl FnOnce(&mut Pipeline)) -> Result<()> {
		let mut pipeline = redis::pipe();
		build(&mut pipeline);

		self.with_connection(|connection| pipeline.query::<()>(connection))?;
		self.metrics.record_redis_write();

		Ok(())
	}

	fn with_connection<T>(&self, run: impl Fn(&mut Connection) -> redis::RedisResult<T>) -> Result<T> {
		let mut connection = self.connection.lock().expect("Redis connection mutex poisoned");

		match run(&mut connection) {
			Ok(value) => Ok(value),

			// The connection may have dropped (e.g. Redis restarted) - reconnect once and retry.
			Err(_) => {
				*connection = self.client.get_connection().context("Unable to reconnect to Redis")?;
				run(&mut connection).context("Redis command failed after reconnecting")
			}
		}
	}
}

fn now_epoch() -> i64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_secs() as i64).unwrap_or(0)
}

// Darwin correlates a forecast/loading update back to a specific calling point by TIPLOC plus its planned working times, since a TIPLOC can appear more than once in a schedule (loops reversals).
fn location_id(
	tiploc: &str,
	working_arrival_time: Option<&str>,
	working_departure_time: Option<&str>,
	pass_time: Option<&str>,
) -> String {
	format!(
		"{tiploc}_{}_{}_{}",
		normalize_time(working_arrival_time),
		normalize_time(working_departure_time),
		normalize_time(pass_time),
	)
}

// "19:47" -> "194700", "19:47:30" -> "194730", missing -> "XXXXXX"
fn normalize_time(value: Option<&str>) -> String {
	let Some(value) = value else {
		return "XXXXXX".to_string();
	};

	let digits: String = value.chars().filter(char::is_ascii_digit).collect();

	match digits.len() {
		6 => digits,
		4 => format!("{digits}00"),
		_ => "XXXXXX".to_string(),
	}
}

fn push_time_data(fields: &mut Fields, prefix: &str, data: &PushPortForecastTimeData) {
	field_opt(fields, &format!("{prefix}_estimated"), &data.estimate);
	field_opt(fields, &format!("{prefix}_actual"), &data.actual);
	field_opt(fields, &format!("{prefix}_working_estimated"), &data.working_estimate);
	field_opt(fields, &format!("{prefix}_minimum_estimated"), &data.minimum_estimate);
	field_bool(fields, &format!("{prefix}_is_delayed"), data.delayed);
	field_bool(fields, &format!("{prefix}_is_actual_removed"), data.actual_remove);
	field_bool(fields, &format!("{prefix}_is_estimate_unknown"), data.estimate_unknown);
	field_opt(fields, &format!("{prefix}_source_system"), &data.source_system);
	field_opt(fields, &format!("{prefix}_source_cis_code"), &data.source_cis_code);
}

fn push_platform_data(fields: &mut Fields, platform: &PushPortForecastPlatformData) {
	field(fields, "platform_value", platform.value.clone());
	field_bool(fields, "platform_is_suppressed", platform.suppress);
	field_bool(fields, "platform_is_confirmed", platform.confirm);
	field_bool(fields, "platform_is_cis_suppressed", platform.cis_suppress);

	if let Some(source) = &platform.source {
		field(fields, "platform_source", format!("{source:?}"));
	}
}

fn field(fields: &mut Fields, name: &str, value: String) {
	fields.push((name.to_string(), value));
}

fn field_opt(fields: &mut Fields, name: &str, value: &Option<String>) {
	if let Some(value) = value {
		fields.push((name.to_string(), value.clone()));
	}
}

fn field_bool(fields: &mut Fields, name: &str, value: bool) {
	fields.push((
		name.to_string(),
		if value {
			"1"
		} else {
			"0"
		}
		.to_string(),
	));
}

fn field_num(fields: &mut Fields, name: &str, value: impl ToString) {
	fields.push((name.to_string(), value.to_string()));
}
