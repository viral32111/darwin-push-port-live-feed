use anyhow::{Result, anyhow, bail};
use quick_xml::{Reader, events::Event};

#[derive(Debug)]
pub struct PushPort {
	pub timestamp: String, // ISO-8601
	pub version: String,   // Always '16.0'

	pub update: PushPortUpdate,
}

#[derive(Debug)]
pub struct PushPortUpdate {
	pub kind: PushPortUpdateKind,

	pub origin: Option<PushPortUpdateOrigin>,

	pub request_id: Option<String>,
	pub request_source: Option<String>,

	pub events: Vec<PushPortEvent>,
}

#[derive(Debug, PartialEq)]
pub enum PushPortUpdateKind {
	Live,     // <uR> - real-time update (sent continuously)
	Snapshot, // <sR> - snapshot replay (sent after feed discontinuity)
}

#[derive(Debug, PartialEq)]
pub enum PushPortUpdateOrigin {
	CIS,
	Darwin,
	TD,
	TRUST,
}

impl PushPortUpdateOrigin {
	fn parse(str: &str) -> Option<Self> {
		match str.to_lowercase().as_str() {
			"cis" => Some(Self::CIS),
			"darwin" => Some(Self::Darwin),
			"td" => Some(Self::TD),
			"trust" => Some(Self::TRUST),

			_ => None,
		}
	}
}

#[derive(Debug)]
pub enum PushPortEvent {
	TrainStatus(PushPortTrainStatus), // <TS> - Real-time train status / forecast update
	Schedule(PushPortSchedule),       // <schedule> - Full or partial schedule push
	Deactivated {
		rid: String,
	},    // <deactivated> - Schedule withdrawn from Darwin
	Association(PushPortAssociation), // <association> - Join, split, link, or next-working association
	StationMessage(PushPortStationMessage), // <OW> - Station message
	TrainAlert(PushPortTrainAlert),   // <trainAlert> - Passenger/staff/operations alert
	TrainOrder(PushPortTrainOrder),   // <trainOrder> - Expected calling order at a station platform
	TrackingId(PushPortTrackingID),   // <trackingID> - Corrected headcode for a berth
	Alarm(PushPortAlarm),             // <alarm> - Internal alarm set or cleared
	TimetableId(PushPortTimetableID), // <TimeTableId> - Notifies that a new timetable (reference data file) is available
	FormationLoading(PushPortFormationLoading), // <formationLoading> - Estimated per-coach passenger loading for a formation at a location
	ScheduleFormations(PushPortScheduleFormations), // <scheduleFormations> - Coach composition (class, facilities) for a schedule's formation(s)
}

#[derive(Debug)]
pub struct PushPortTrainStatus {
	pub darwin_timetable_id: String,

	pub schedule_uid: String,
	pub schedule_start_date: String, // YYYY-MM-DD

	pub is_reverse_formation: bool,

	pub delay_reason_code: Option<PushPortDisruptionReason>, // See codes in reference data

	pub locations: Vec<PushPortForecastLocation>,
}

#[derive(Debug)]
pub struct PushPortForecastLocation {
	pub tiploc: String,

	pub working_arrival_time: Option<String>,
	pub working_departure_time: Option<String>,

	pub public_arrival_time: Option<String>,
	pub public_departure_time: Option<String>,

	pub pass_time: Option<String>,

	pub arrive: Option<PushPortForecastTimeData>,
	pub depart: Option<PushPortForecastTimeData>,
	pub pass: Option<PushPortForecastTimeData>,

	pub platform: Option<PushPortForecastPlatformData>,

	pub length: Option<u16>, // Number of carriages
	pub suppress: bool,      // Service is suppressed from public displays at this location.
	pub detach_front: bool,  // Stock will be detached from the front of the train here.
}

#[derive(Debug)]
pub struct PushPortForecastTimeData {
	pub delayed: bool, // Whether the delay is unknown

	pub estimate: Option<String>,
	pub actual: Option<String>, // Only set once the train has arrived/departed/passed this location

	pub working_estimate: Option<String>, // Set only when it differs from the public estimate
	pub minimum_estimate: Option<String>, // Manually applied lower bound

	pub actual_remove: bool, // Swap actual for estimate
	pub estimate_unknown: bool,

	pub source_system: Option<String>,   // Darwin, TD, CIS, Trust, etc.
	pub source_cis_code: Option<String>, // See reference data for CIS codes
}

#[derive(Debug)]
pub struct PushPortForecastPlatformData {
	pub value: String,                                  // e.g. 1, 2A, B
	pub source: Option<PushPortForecastPlatformSource>, // P = Planned / A = Automatic / M = Manual

	pub suppress: bool, // Do not show on public displays
	pub confirm: bool,  // Confirmed by ToC

	pub cis_suppress: bool, // Suppressed by CIS or Darwin
}

#[derive(Debug, PartialEq)]
pub enum PushPortForecastPlatformSource {
	Planned,
	Automatic,
	Manual,
}

impl PushPortForecastPlatformSource {
	fn parse(str: &str) -> Option<Self> {
		match str.to_uppercase().as_str() {
			"A" => Some(Self::Automatic),
			"M" => Some(Self::Manual),
			"P" => Some(Self::Planned),

			_ => None,
		}
	}
}

#[derive(Debug)]
pub struct PushPortSchedule {
	pub darwin_id: String,

	pub schedule_uid: String,
	pub schedule_start_date: String,

	pub headcode: String,
	pub operator_code: String,

	pub category: String,
	pub status: String,

	pub is_active: bool,

	pub is_passenger: bool,
	pub is_charter: bool,
	pub is_deleted: bool,

	pub cancel_reason_code: Option<PushPortDisruptionReason>,

	pub locations: Vec<PushPortScheduleLocation>,
}

#[derive(Debug)]
pub struct PushPortScheduleLocation {
	pub kind: PushPortScheduleLocationKind,

	pub tiploc: String,
	pub platform: Option<String>,

	pub flags: Option<String>,
	pub original_flags: Option<String>,

	pub working_arrival_time: Option<String>,
	pub working_departure_time: Option<String>,

	pub public_arrival_time: Option<String>,
	pub public_departure_time: Option<String>,

	pub pass_time: Option<String>,

	pub is_cancelled: bool,
	pub false_destination: Option<String>,
	pub reroute_delay: Option<i16>,
}

#[derive(Debug, PartialEq)]
pub enum PushPortScheduleLocationKind {
	Origin,
	CallingPoint,
	PassingPoint,
	Destination,

	OperationalOrigin,
	OperationalIntermediate,
	OperationalDestination,
}

#[derive(Debug)]
pub struct PushPortAssociation {
	pub tiploc: String,

	pub category: PushPortAssociationCategory,

	pub is_cancelled: bool,
	pub is_deleted: bool,

	pub main: PushPortAssociationService,
	pub associated: PushPortAssociationService,
}

#[derive(Debug)]
pub struct PushPortAssociationService {
	pub darwin_id: String,

	pub working_arrival_time: Option<String>,
	pub working_departure_time: Option<String>,

	pub public_arrival_time: Option<String>,
	pub public_departure_time: Option<String>,

	pub pass_time: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum PushPortAssociationCategory {
	Join,
	Split,
	Linked,
	NextWorking,
}

impl PushPortAssociationCategory {
	fn parse(str: &str) -> Option<Self> {
		match str.to_uppercase().as_str() {
			"JJ" => Some(Self::Join),
			"VV" => Some(Self::Split),
			"LK" => Some(Self::Linked),
			"NP" => Some(Self::NextWorking),
			_ => None,
		}
	}
}

#[derive(Debug)]
pub struct PushPortStationMessage {
	pub id: i32,

	pub category: PushPortMessageCategory,
	pub severity: u8,
	pub suppress: bool,

	pub stations: Vec<String>, // CRS codes

	pub message: String, // Plain text - inner markup (e.g. <a> links) is stripped, not preserved
}

#[derive(Debug, PartialEq)]
pub enum PushPortMessageCategory {
	Train,
	Station,
	Connections,
	System,
	Misc,
	PriorTrains,
	Prior,
}

impl PushPortMessageCategory {
	fn parse(str: &str) -> Option<Self> {
		match str.to_lowercase().as_str() {
			"train" => Some(Self::Train),
			"station" => Some(Self::Station),
			"connections" => Some(Self::Connections),
			"system" => Some(Self::System),
			"misc" => Some(Self::Misc),
			"priortrains" => Some(Self::PriorTrains),
			"priorother" => Some(Self::Prior),
			_ => None,
		}
	}
}

#[derive(Debug)]
pub struct PushPortTrainAlert {
	pub id: String,
	pub kind: PushPortAlertKind,

	pub services: Vec<PushPortAlertService>,

	pub send_by_sms: bool,
	pub send_by_email: bool,
	pub send_by_twitter: bool,

	pub source: String,
	pub text: String,
	pub audience: PushPortAlertAudience,

	pub copied_from_id: Option<String>,
	pub copied_from_source: Option<String>,
}

#[derive(Debug)]
pub struct PushPortAlertService {
	pub darwin_id: Option<String>,

	pub schedule_uid: Option<String>,
	pub schedule_start_date: Option<String>,

	pub locations: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum PushPortAlertAudience {
	Public,
	Staff,
	Operations,
}

impl PushPortAlertAudience {
	fn parse(str: &str) -> Option<Self> {
		match str.to_lowercase().as_str() {
			"customer" => Some(Self::Public),
			"staff" => Some(Self::Staff),
			"operations" => Some(Self::Operations),
			_ => None,
		}
	}
}

#[derive(Debug, PartialEq)]
pub enum PushPortAlertKind {
	Normal,
	Force,
}

impl PushPortAlertKind {
	fn parse(str: &str) -> Option<Self> {
		match str.to_lowercase().as_str() {
			"normal" => Some(Self::Normal),
			"forced" => Some(Self::Force),
			_ => None,
		}
	}
}

#[derive(Debug)]
pub struct PushPortTrainOrder {
	pub tiploc: String,
	pub crs: String,
	pub platform: String,
	pub action: PushPortTrainOrderAction,
}

#[derive(Debug)]
pub enum PushPortTrainOrderAction {
	Set(PushPortTrainOrderData),
	Clear,
}

#[derive(Debug)]
pub struct PushPortTrainOrderData {
	pub first: PushPortTrainOrderItem,
	pub second: Option<PushPortTrainOrderItem>,
	pub third: Option<PushPortTrainOrderItem>,
}

#[derive(Debug)]
pub enum PushPortTrainOrderItem {
	Darwin {
		darwin_id: String,

		working_arrival_time: Option<String>,
		working_departure_time: Option<String>,

		pass_time: Option<String>,
	},
	Unknown(String),
}

#[derive(Debug)]
pub struct PushPortTrackingID {
	pub area: String,  // 2 characters
	pub berth: String, // 4 characters

	pub current: String,
	pub correction: String,
}

#[derive(Debug)]
pub struct PushPortAlarm {
	pub action: PushPortAlarmAction,
}

#[derive(Debug)]
pub enum PushPortAlarmAction {
	Set(PushPortAlarmData),
	Clear(String),
}

#[derive(Debug)]
pub struct PushPortAlarmData {
	pub id: String,
	pub kind: PushPortAlarmKind,
}

#[derive(Debug)]
pub enum PushPortAlarmKind {
	AreaFail {
		area: String,
	},
	FeedFail,
	TyrellFeedFail,
}

#[derive(Debug)]
pub struct PushPortTimetableID {
	pub id: String,

	pub timetable_file: String,
	pub reference_file: String,
}

#[derive(Debug)]
pub struct PushPortFormationLoading {
	pub darwin_id: String, // rid
	pub formation_id: String,
	pub tiploc: String,

	pub working_arrival_time: Option<String>,
	pub working_departure_time: Option<String>,
	pub pass_time: Option<String>,

	pub coaches: Vec<PushPortCoachLoading>,
}

#[derive(Debug)]
pub struct PushPortCoachLoading {
	pub coach_number: String,

	pub source: Option<String>,        // Darwin, TD, CIS, Trust, etc.
	pub source_system: Option<String>, // See reference data for CIS codes

	pub percentage: Option<u8>, // 0-100
}

#[derive(Debug)]
pub struct PushPortScheduleFormations {
	pub darwin_id: String, // rid

	pub formations: Vec<PushPortFormation>,
}

#[derive(Debug)]
pub struct PushPortFormation {
	pub formation_id: String, // fid

	pub coaches: Vec<PushPortFormationCoach>,
}

#[derive(Debug)]
pub struct PushPortFormationCoach {
	pub coach_number: String,
	pub coach_class: Option<String>,

	pub toilet_status: Option<String>, // InService / NotInService / Unknown
	pub toilet_type: Option<String>,   // None / Standard / Accessible
}

#[derive(Debug)]
pub struct PushPortDisruptionReason {
	pub code: i16,
	pub tiploc: Option<String>,
	pub is_near: bool,
}

pub fn parse(xml: &str) -> Result<PushPort> {
	let mut reader = Reader::from_str(xml);
	reader.config_mut().trim_text(true);

	let mut buffer = Vec::new();

	let (timestamp, version) = loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"Pport" => {
				let timestamp = str_attr(element, b"ts")?.ok_or_else(|| anyhow!("<Pport> missing 'ts' attribute"))?;
				let version = str_attr(element, b"version")?.unwrap_or_default();

				break (timestamp, version);
			}

			Event::Eof => bail!("Root element is not <Pport>"),

			_ => {}
		}

		buffer.clear();
	};

	buffer.clear();

	let update = loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) => match local_name(&element.name()) {
				b"uR" | b"sR" => {
					let kind = if local_name(&element.name()) == b"uR" {
						PushPortUpdateKind::Live
					} else {
						PushPortUpdateKind::Snapshot
					};

					let update_origin = str_attr(element, b"updateOrigin")?;
					let request_source = str_attr(element, b"requestSource")?;
					let request_id = str_attr(element, b"requestID")?;

					let tag = local_name(&element.name()).to_vec();

					buffer.clear();

					let events = parse_events(&mut reader, &tag)?;

					break PushPortUpdate {
						kind,
						origin: update_origin.as_deref().and_then(PushPortUpdateOrigin::parse),
						request_source,
						request_id,
						events,
					};
				}

				b"TimeTableId" => {
					let id_str = str_attr(element, b"ttfile")?.unwrap_or_default();
					let ref_str = str_attr(element, b"ttreffile")?.unwrap_or_default();

					buffer.clear();

					let mut text_id = String::new();
					loop {
						match reader.read_event_into(&mut buffer)? {
							Event::Text(ref text) => text_id = text.decode()?.into_owned(),
							Event::End(ref element) if local_name(&element.name()) == b"TimeTableId" => break,

							Event::Eof => break,

							_ => {}
						}

						buffer.clear();
					}

					return Ok(PushPort {
						timestamp,
						version,
						update: PushPortUpdate {
							kind: PushPortUpdateKind::Live,
							origin: None,
							request_source: None,
							request_id: None,
							events: vec![PushPortEvent::TimetableId(PushPortTimetableID {
								id: text_id,
								timetable_file: id_str,
								reference_file: ref_str,
							})],
						},
					});
				}

				_ => {
					let tag = local_name(&element.name()).to_vec();
					buffer.clear();
					skip_element(&mut reader, &tag)?;
				}
			},

			Event::Empty(_) => {}
			Event::End(_) | Event::Eof => bail!("<Pport> has no recognisable child element"),

			_ => {}
		}

		buffer.clear();
	};

	Ok(PushPort {
		timestamp,
		version,
		update,
	})
}

fn parse_events(reader: &mut Reader<&[u8]>, tag: &[u8]) -> Result<Vec<PushPortEvent>> {
	let mut events = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) => match local_name(&element.name()) {
				b"TS" => {
					let rid = str_attr(element, b"rid")?.ok_or_else(|| anyhow!("<TS> missing 'rid'"))?;
					let uid = str_attr(element, b"uid")?.unwrap_or_default();
					let ssd = str_attr(element, b"ssd")?.unwrap_or_default();
					let is_reverse_formation = bool_attr(element, b"isReverseFormation");

					buffer.clear();

					let (late_reason, locations) = parse_train_schedule(reader)?;

					events.push(PushPortEvent::TrainStatus(PushPortTrainStatus {
						darwin_timetable_id: rid,
						schedule_uid: uid,
						schedule_start_date: ssd,
						is_reverse_formation,
						delay_reason_code: late_reason,
						locations,
					}));
				}

				b"schedule" => {
					let s = parse_schedule(reader, element)?;
					events.push(PushPortEvent::Schedule(s));
				}

				b"association" => {
					let a = parse_association(reader, element)?;
					events.push(PushPortEvent::Association(a));
				}

				b"OW" => {
					let sm = parse_station_message(reader, element)?;
					events.push(PushPortEvent::StationMessage(sm));
				}

				b"trainAlert" => {
					buffer.clear();
					let ta = parse_train_alert(reader)?;
					events.push(PushPortEvent::TrainAlert(ta));
				}

				b"trainOrder" => {
					let to = parse_train_order(reader, element)?;
					events.push(PushPortEvent::TrainOrder(to));
				}

				b"trackingID" => {
					buffer.clear();
					let tid = parse_tracking_id(reader)?;
					events.push(PushPortEvent::TrackingId(tid));
				}

				b"alarm" => {
					buffer.clear();
					let alm = parse_alarm(reader)?;
					events.push(PushPortEvent::Alarm(alm));
				}

				b"formationLoading" => {
					let fl = parse_formation_loading(reader, element)?;
					events.push(PushPortEvent::FormationLoading(fl));
				}

				b"scheduleFormations" => {
					let sf = parse_schedule_formations(reader, element)?;
					events.push(PushPortEvent::ScheduleFormations(sf));
				}

				_ => {
					let tag = local_name(&element.name()).to_vec();
					buffer.clear();
					skip_element(reader, &tag)?;
				}
			},

			Event::Empty(ref element) => match local_name(&element.name()) {
				b"TS" => {
					if let Some(rid) = str_attr(element, b"rid")? {
						events.push(PushPortEvent::TrainStatus(PushPortTrainStatus {
							darwin_timetable_id: rid,
							schedule_uid: str_attr(element, b"uid")?.unwrap_or_default(),
							schedule_start_date: str_attr(element, b"ssd")?.unwrap_or_default(),
							is_reverse_formation: bool_attr(element, b"isReverseFormation"),
							delay_reason_code: None,
							locations: vec![],
						}));
					}
				}

				b"deactivated" => {
					if let Some(rid) = str_attr(element, b"rid")? {
						events.push(PushPortEvent::Deactivated {
							rid,
						});
					}
				}

				_ => {}
			},

			Event::End(ref element) if local_name(&element.name()) == tag => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(events)
}

fn parse_train_schedule(
	reader: &mut Reader<&[u8]>,
) -> Result<(Option<PushPortDisruptionReason>, Vec<PushPortForecastLocation>)> {
	let mut late_reason: Option<PushPortDisruptionReason> = None;
	let mut locations: Vec<PushPortForecastLocation> = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"LateReason" => {
				buffer.clear();
				late_reason = Some(parse_disruption_reason(reader, b"LateReason")?);
			}

			Event::Start(ref element) if local_name(&element.name()) == b"Location" => {
				let tiploc = str_attr(element, b"tpl")?.unwrap_or_default();
				let wta = str_attr(element, b"wta")?;
				let wtd = str_attr(element, b"wtd")?;
				let wtp = str_attr(element, b"wtp")?;
				let pta = str_attr(element, b"pta")?;
				let ptd = str_attr(element, b"ptd")?;

				buffer.clear();

				locations.push(parse_forecast_location(reader, tiploc, wta, wtd, wtp, pta, ptd)?);
			}

			Event::End(ref element) if local_name(&element.name()) == b"TS" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok((late_reason, locations))
}

fn parse_forecast_location(
	reader: &mut Reader<&[u8]>,
	tiploc: String,
	working_arrival: Option<String>,
	working_departure: Option<String>,
	working_pass: Option<String>,
	public_arrival: Option<String>,
	public_departure: Option<String>,
) -> Result<PushPortForecastLocation> {
	let mut arr: Option<PushPortForecastTimeData> = None;
	let mut dep: Option<PushPortForecastTimeData> = None;
	let mut pass: Option<PushPortForecastTimeData> = None;

	let mut plat: Option<PushPortForecastPlatformData> = None;
	let mut length: Option<u16> = None;

	let mut suppress = false;
	let mut detach_front = false;

	let mut should_read_platform = false;
	let mut should_read_length = false;
	let mut should_read_suppr = false;
	let mut should_read_detach_front = false;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Empty(ref element) | Event::Start(ref element) => match local_name(&element.name()) {
				b"arr" => arr = Some(parse_time_data(element)?),
				b"dep" => dep = Some(parse_time_data(element)?),
				b"pass" => pass = Some(parse_time_data(element)?),
				b"plat" => {
					plat = Some(parse_platform_data(element)?);
					should_read_platform = true;
				}
				b"length" => should_read_length = true,
				b"suppr" => should_read_suppr = true,
				b"detachFront" => should_read_detach_front = true,
				_ => {}
			},

			Event::Text(ref text) => {
				let text = text.decode()?.into_owned();
				let text = text.trim();

				if should_read_platform {
					if let Some(ref mut p) = plat {
						p.value = text.to_string();
					}
					should_read_platform = false;
				} else if should_read_length {
					length = text.parse().ok();
					should_read_length = false;
				} else if should_read_suppr {
					suppress = text == "true";
					should_read_suppr = false;
				} else if should_read_detach_front {
					detach_front = text == "true";
					should_read_detach_front = false;
				}
			}

			Event::End(ref element) => match local_name(&element.name()) {
				b"plat" => should_read_platform = false,
				b"length" => should_read_length = false,
				b"suppr" => should_read_suppr = false,
				b"detachFront" => should_read_detach_front = false,
				b"Location" => break,
				_ => {}
			},

			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortForecastLocation {
		tiploc,

		working_arrival_time: working_arrival,
		working_departure_time: working_departure,

		public_arrival_time: public_arrival,
		public_departure_time: public_departure,

		pass_time: working_pass,

		arrive: arr,
		depart: dep,
		pass,

		platform: plat,

		length,
		suppress,
		detach_front,
	})
}

fn parse_time_data(e: &quick_xml::events::BytesStart) -> Result<PushPortForecastTimeData> {
	Ok(PushPortForecastTimeData {
		estimate: str_attr(e, b"et")?,
		working_estimate: str_attr(e, b"wet")?,
		minimum_estimate: str_attr(e, b"etmin")?,
		actual: str_attr(e, b"at")?,
		actual_remove: bool_attr(e, b"atRemoved"),
		delayed: bool_attr(e, b"delayed"),
		estimate_unknown: bool_attr(e, b"etUnknown"),
		source_system: str_attr(e, b"src")?,
		source_cis_code: str_attr(e, b"srcInst")?,
	})
}

fn parse_platform_data(e: &quick_xml::events::BytesStart) -> Result<PushPortForecastPlatformData> {
	Ok(PushPortForecastPlatformData {
		value: String::new(),
		suppress: bool_attr(e, b"platsup"),
		cis_suppress: bool_attr(e, b"cisPlatsup"),
		source: str_attr(e, b"platsrc")?
			.as_deref()
			.and_then(PushPortForecastPlatformSource::parse)
			.or(Some(PushPortForecastPlatformSource::Planned)),
		confirm: bool_attr(e, b"conf"),
	})
}

fn parse_schedule(reader: &mut Reader<&[u8]>, open: &quick_xml::events::BytesStart) -> Result<PushPortSchedule> {
	let rid = str_attr(open, b"rid")?.ok_or_else(|| anyhow!("<schedule> missing 'rid'"))?;
	let uid = str_attr(open, b"uid")?.unwrap_or_default();
	let train_id = str_attr(open, b"trainId")?.unwrap_or_default();
	let ssd = str_attr(open, b"ssd")?.unwrap_or_default();
	let toc = str_attr(open, b"toc")?.unwrap_or_default();
	let status = str_attr(open, b"status")?.unwrap_or_else(|| "P".into());
	let train_category = str_attr(open, b"trainCat")?.unwrap_or_else(|| "OO".into());
	let is_passenger_service = str_attr(open, b"isPassengerSvc")?.as_deref() != Some("false");
	let is_active = str_attr(open, b"isActive")?.as_deref() != Some("false");
	let is_deleted = bool_attr(open, b"deleted");
	let is_charter = bool_attr(open, b"isCharter");

	let mut cancel_reason: Option<PushPortDisruptionReason> = None;
	let mut locations: Vec<PushPortScheduleLocation> = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Empty(ref element) | Event::Start(ref element) => {
				let kind = match local_name(&element.name()) {
					b"OR" => Some(PushPortScheduleLocationKind::Origin),
					b"OPOR" => Some(PushPortScheduleLocationKind::OperationalOrigin),
					b"IP" => Some(PushPortScheduleLocationKind::CallingPoint),
					b"OPIP" => Some(PushPortScheduleLocationKind::OperationalIntermediate),
					b"PP" => Some(PushPortScheduleLocationKind::PassingPoint),
					b"DT" => Some(PushPortScheduleLocationKind::Destination),
					b"OPDT" => Some(PushPortScheduleLocationKind::OperationalDestination),
					b"cancelReason" => {
						buffer.clear();
						cancel_reason = Some(parse_disruption_reason(reader, b"cancelReason")?);
						continue;
					}
					_ => None,
				};

				if let Some(kind) = kind {
					locations.push(PushPortScheduleLocation {
						kind,
						tiploc: str_attr(element, b"tpl")?.unwrap_or_default(),
						flags: str_attr(element, b"act")?,
						original_flags: str_attr(element, b"planAct")?,
						is_cancelled: bool_attr(element, b"can"),
						platform: str_attr(element, b"plat")?,
						working_arrival_time: str_attr(element, b"wta")?,
						working_departure_time: str_attr(element, b"wtd")?,
						pass_time: str_attr(element, b"wtp")?,
						public_arrival_time: str_attr(element, b"pta")?,
						public_departure_time: str_attr(element, b"ptd")?,
						reroute_delay: str_attr(element, b"rdelay")?.and_then(|s| s.parse().ok()),
						false_destination: str_attr(element, b"fd")?,
					});
				}
			}

			Event::End(ref element) if local_name(&element.name()) == b"schedule" => break,
			Event::Eof => break,

			_ => {}
		}
		buffer.clear();
	}

	Ok(PushPortSchedule {
		darwin_id: rid,
		schedule_uid: uid,
		headcode: train_id,
		schedule_start_date: ssd,
		operator_code: toc,
		status,
		category: train_category,
		is_passenger: is_passenger_service,
		is_active,
		is_deleted,
		is_charter,
		cancel_reason_code: cancel_reason,
		locations,
	})
}

fn parse_association(reader: &mut Reader<&[u8]>, open: &quick_xml::events::BytesStart) -> Result<PushPortAssociation> {
	let tiploc = str_attr(open, b"tiploc")?.unwrap_or_default();
	let category = str_attr(open, b"category")?
		.and_then(|s| PushPortAssociationCategory::parse(&s))
		.ok_or_else(|| anyhow!("<association> missing or unknown 'category'"))?;
	let is_cancelled = bool_attr(open, b"isCancelled");
	let is_deleted = bool_attr(open, b"isDeleted");

	let mut main: Option<PushPortAssociationService> = None;
	let mut associated: Option<PushPortAssociationService> = None;
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Empty(ref element) | Event::Start(ref element) => match local_name(&element.name()) {
				b"main" | b"assoc" => {
					let svc = PushPortAssociationService {
						darwin_id: str_attr(element, b"rid")?.unwrap_or_default(),
						working_arrival_time: str_attr(element, b"wta")?,
						working_departure_time: str_attr(element, b"wtd")?,
						pass_time: str_attr(element, b"wtp")?,
						public_arrival_time: str_attr(element, b"pta")?,
						public_departure_time: str_attr(element, b"ptd")?,
					};

					if local_name(&element.name()) == b"main" {
						main = Some(svc);
					} else {
						associated = Some(svc);
					}
				}

				_ => {}
			},

			Event::End(ref element) if local_name(&element.name()) == b"association" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortAssociation {
		tiploc,
		category,
		is_cancelled,
		is_deleted,
		main: main.ok_or_else(|| anyhow!("<association> missing <main>"))?,
		associated: associated.ok_or_else(|| anyhow!("<association> missing <assoc>"))?,
	})
}

fn parse_station_message(
	reader: &mut Reader<&[u8]>,
	open: &quick_xml::events::BytesStart,
) -> Result<PushPortStationMessage> {
	let id = str_attr(open, b"id")?.and_then(|s| s.parse().ok()).unwrap_or(0);
	let cat = str_attr(open, b"cat")?
		.and_then(|s| PushPortMessageCategory::parse(&s))
		.ok_or_else(|| anyhow!("<OW> missing or unknown 'cat'"))?;
	let sev = str_attr(open, b"sev")?.and_then(|s| s.parse().ok()).unwrap_or(0);
	let suppress = bool_attr(open, b"suppress");

	let mut stations: Vec<String> = Vec::new();
	let mut message = String::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Empty(ref element) if local_name(&element.name()) == b"Station" => {
				if let Some(crs) = str_attr(element, b"crs")? {
					stations.push(crs);
				}
			}

			Event::Start(ref element) if local_name(&element.name()) == b"Msg" => {
				buffer.clear();
				message = collect_inner_text(reader, b"Msg")?;
			}

			Event::End(ref element) if local_name(&element.name()) == b"OW" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortStationMessage {
		id,
		category: cat,
		severity: sev,
		suppress,
		stations,
		message,
	})
}

fn parse_train_alert(reader: &mut Reader<&[u8]>) -> Result<PushPortTrainAlert> {
	let mut alert_id = String::new();
	let mut services: Vec<PushPortAlertService> = Vec::new();
	let mut send_sms = false;
	let mut send_email = false;
	let mut send_twitter = false;
	let mut source = String::new();
	let mut text = String::new();
	let mut audience: Option<PushPortAlertAudience> = None;
	let mut kind: Option<PushPortAlertKind> = None;
	let mut copied_id: Option<String> = None;
	let mut copied_source: Option<String> = None;

	let mut current_text_target: Option<&'static str> = None;
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) => match local_name(&element.name()) {
				b"AlertID" => current_text_target = Some("AlertID"),
				b"SendAlertBySMS" => current_text_target = Some("SendAlertBySMS"),
				b"SendAlertByEmail" => current_text_target = Some("SendAlertByEmail"),
				b"SendAlertByTwitter" => current_text_target = Some("SendAlertByTwitter"),
				b"Source" => current_text_target = Some("Source"),
				b"AlertText" => current_text_target = Some("AlertText"),
				b"Audience" => current_text_target = Some("Audience"),
				b"AlertType" => current_text_target = Some("AlertType"),
				b"CopiedFromAlertID" => current_text_target = Some("CopiedFromAlertID"),
				b"CopiedFromSource" => current_text_target = Some("CopiedFromSource"),
				b"AlertServices" => {
					buffer.clear();
					services = parse_alert_services(reader)?;
				}

				_ => {}
			},

			Event::Text(ref txt) => {
				let value = txt.decode()?.into_owned();

				match current_text_target {
					Some("AlertID") => alert_id = value,
					Some("SendAlertBySMS") => send_sms = value.trim() == "true",
					Some("SendAlertByEmail") => send_email = value.trim() == "true",
					Some("SendAlertByTwitter") => send_twitter = value.trim() == "true",
					Some("Source") => source = value,
					Some("AlertText") => text = value,
					Some("Audience") => audience = PushPortAlertAudience::parse(value.trim()),
					Some("AlertType") => kind = PushPortAlertKind::parse(value.trim()),
					Some("CopiedFromAlertID") => copied_id = Some(value),
					Some("CopiedFromSource") => copied_source = Some(value),
					_ => {}
				}

				current_text_target = None;
			}

			Event::End(ref element) if local_name(&element.name()) == b"trainAlert" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortTrainAlert {
		id: alert_id,
		services,
		send_by_sms: send_sms,
		send_by_email: send_email,
		send_by_twitter: send_twitter,
		source,
		text,
		audience: audience.unwrap_or(PushPortAlertAudience::Public),
		kind: kind.unwrap_or(PushPortAlertKind::Normal),
		copied_from_id: copied_id,
		copied_from_source: copied_source,
	})
}

fn parse_alert_services(reader: &mut Reader<&[u8]>) -> Result<Vec<PushPortAlertService>> {
	let mut services = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"AlertService" => {
				let rid = str_attr(element, b"RID")?;
				let uid = str_attr(element, b"UID")?;
				let ssd = str_attr(element, b"SSD")?;

				buffer.clear();

				let mut locations = Vec::new();
				loop {
					match reader.read_event_into(&mut buffer)? {
						Event::Start(ref element) if local_name(&element.name()) == b"Location" => {
							buffer.clear();
							let loc = collect_inner_text(reader, b"Location")?;
							locations.push(loc);
						}

						Event::End(ref element) if local_name(&element.name()) == b"AlertService" => break,
						Event::Eof => break,

						_ => {}
					}

					buffer.clear();
				}

				services.push(PushPortAlertService {
					darwin_id: rid,
					schedule_uid: uid,
					schedule_start_date: ssd,
					locations,
				});
			}

			Event::End(ref element) if local_name(&element.name()) == b"AlertServices" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(services)
}

fn parse_train_order(reader: &mut Reader<&[u8]>, open: &quick_xml::events::BytesStart) -> Result<PushPortTrainOrder> {
	let tiploc = str_attr(open, b"tiploc")?.unwrap_or_default();
	let crs = str_attr(open, b"crs")?.unwrap_or_default();
	let platform = str_attr(open, b"platform")?.unwrap_or_default();
	let mut action: Option<PushPortTrainOrderAction> = None;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"set" => {
				buffer.clear();
				action = Some(PushPortTrainOrderAction::Set(parse_train_order_data(reader)?));
			}

			Event::Empty(ref element) if local_name(&element.name()) == b"clear" => {
				action = Some(PushPortTrainOrderAction::Clear);
			}

			Event::Start(ref element) if local_name(&element.name()) == b"clear" => {
				action = Some(PushPortTrainOrderAction::Clear);
				buffer.clear();
				skip_element(reader, b"clear")?;
			}

			Event::End(ref element) if local_name(&element.name()) == b"trainOrder" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortTrainOrder {
		tiploc,
		crs,
		platform,
		action: action.unwrap_or(PushPortTrainOrderAction::Clear),
	})
}

fn parse_train_order_data(reader: &mut Reader<&[u8]>) -> Result<PushPortTrainOrderData> {
	let mut first: Option<PushPortTrainOrderItem> = None;
	let mut second: Option<PushPortTrainOrderItem> = None;
	let mut third: Option<PushPortTrainOrderItem> = None;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) => {
				let slot = match local_name(&element.name()) {
					b"first" => 1,
					b"second" => 2,
					b"third" => 3,
					_ => 0,
				};

				if slot > 0 {
					buffer.clear();
					let item = parse_train_order_item(reader)?;
					match slot {
						1 => first = Some(item),
						2 => second = Some(item),
						_ => third = Some(item),
					}
				}
			}

			Event::End(ref element) if local_name(&element.name()) == b"set" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortTrainOrderData {
		first: first.ok_or_else(|| anyhow!("<set> missing <first>"))?,
		second,
		third,
	})
}

fn parse_train_order_item(reader: &mut Reader<&[u8]>) -> Result<PushPortTrainOrderItem> {
	let mut item: Option<PushPortTrainOrderItem> = None;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"rid" => {
				let wta = str_attr(element, b"wta")?;
				let wtd = str_attr(element, b"wtd")?;
				let wtp = str_attr(element, b"wtp")?;

				buffer.clear();

				let rid = collect_inner_text(reader, b"rid")?;

				item = Some(PushPortTrainOrderItem::Darwin {
					darwin_id: rid,
					working_arrival_time: wta,
					working_departure_time: wtd,
					pass_time: wtp,
				});
			}

			Event::Start(ref element) if local_name(&element.name()) == b"trainID" => {
				buffer.clear();
				let id = collect_inner_text(reader, b"trainID")?;
				item = Some(PushPortTrainOrderItem::Unknown(id));
			}

			Event::End(_) | Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	item.ok_or_else(|| anyhow!("train order item had no <rid> or <trainID>"))
}

fn parse_tracking_id(reader: &mut Reader<&[u8]>) -> Result<PushPortTrackingID> {
	let mut area = String::new();
	let mut berth = String::new();
	let mut incorrect_train_id = String::new();
	let mut correct_train_id = String::new();

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"berth" => {
				area = str_attr(element, b"area")?.unwrap_or_default();
				buffer.clear();
				berth = collect_inner_text(reader, b"berth")?;
			}

			Event::Start(ref element) if local_name(&element.name()) == b"incorrectTrainID" => {
				buffer.clear();
				incorrect_train_id = collect_inner_text(reader, b"incorrectTrainID")?;
			}

			Event::Start(ref element) if local_name(&element.name()) == b"correctTrainID" => {
				buffer.clear();
				correct_train_id = collect_inner_text(reader, b"correctTrainID")?;
			}

			Event::End(ref element) if local_name(&element.name()) == b"trackingID" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortTrackingID {
		area,
		berth,
		current: incorrect_train_id,
		correction: correct_train_id,
	})
}

fn parse_alarm(reader: &mut Reader<&[u8]>) -> Result<PushPortAlarm> {
	let mut action: Option<PushPortAlarmAction> = None;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"set" => {
				let id = str_attr(element, b"id")?.unwrap_or_default();
				buffer.clear();
				let kind = parse_alarm_data(reader)?;
				action = Some(PushPortAlarmAction::Set(PushPortAlarmData {
					id,
					kind,
				}));
			}

			Event::Start(ref element) if local_name(&element.name()) == b"clear" => {
				buffer.clear();
				let id = collect_inner_text(reader, b"clear")?;
				action = Some(PushPortAlarmAction::Clear(id));
			}

			Event::End(ref element) if local_name(&element.name()) == b"alarm" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortAlarm {
		action: action.ok_or_else(|| anyhow!("<alarm> had no action"))?,
	})
}

fn parse_alarm_data(reader: &mut Reader<&[u8]>) -> Result<PushPortAlarmKind> {
	let mut kind: Option<PushPortAlarmKind> = None;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"tdAreaFail" => {
				buffer.clear();
				let area = collect_inner_text(reader, b"tdAreaFail")?;
				kind = Some(PushPortAlarmKind::AreaFail {
					area,
				});
			}

			Event::Empty(ref element) if local_name(&element.name()) == b"tdFeedFail" => {
				kind = Some(PushPortAlarmKind::FeedFail);
			}
			Event::Start(ref element) if local_name(&element.name()) == b"tdFeedFail" => {
				buffer.clear();
				skip_element(reader, b"tdFeedFail")?;
				kind = Some(PushPortAlarmKind::FeedFail);
			}

			Event::Empty(ref element) if local_name(&element.name()) == b"tyrellFeedFail" => {
				kind = Some(PushPortAlarmKind::TyrellFeedFail);
			}
			Event::Start(ref element) if local_name(&element.name()) == b"tyrellFeedFail" => {
				buffer.clear();
				skip_element(reader, b"tyrellFeedFail")?;
				kind = Some(PushPortAlarmKind::TyrellFeedFail);
			}

			Event::End(ref element) if local_name(&element.name()) == b"set" => break,
			Event::Eof => break,

			_ => {}
		}
		buffer.clear();
	}

	kind.ok_or_else(|| anyhow!("<set> had no alarm type"))
}

fn parse_formation_loading(
	reader: &mut Reader<&[u8]>,
	open: &quick_xml::events::BytesStart,
) -> Result<PushPortFormationLoading> {
	let rid = str_attr(open, b"rid")?.ok_or_else(|| anyhow!("<formationLoading> missing 'rid'"))?;
	let fid = str_attr(open, b"fid")?.unwrap_or_default();
	let tiploc = str_attr(open, b"tpl")?.unwrap_or_default();
	let wta = str_attr(open, b"wta")?;
	let wtd = str_attr(open, b"wtd")?;
	let wtp = str_attr(open, b"wtp")?;

	let mut coaches: Vec<PushPortCoachLoading> = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"loading" => {
				let coach_number = str_attr(element, b"coachNumber")?.unwrap_or_default();
				let source = str_attr(element, b"src")?;
				let source_system = str_attr(element, b"srcInst")?;

				buffer.clear();

				let text = collect_inner_text(reader, b"loading")?;

				coaches.push(PushPortCoachLoading {
					coach_number,
					source,
					source_system,
					percentage: text.trim().parse().ok(),
				});
			}

			Event::Empty(ref element) if local_name(&element.name()) == b"loading" => {
				coaches.push(PushPortCoachLoading {
					coach_number: str_attr(element, b"coachNumber")?.unwrap_or_default(),
					source: str_attr(element, b"src")?,
					source_system: str_attr(element, b"srcInst")?,
					percentage: None,
				});
			}

			Event::End(ref element) if local_name(&element.name()) == b"formationLoading" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortFormationLoading {
		darwin_id: rid,
		formation_id: fid,
		tiploc,
		working_arrival_time: wta,
		working_departure_time: wtd,
		pass_time: wtp,
		coaches,
	})
}

fn parse_schedule_formations(
	reader: &mut Reader<&[u8]>,
	open: &quick_xml::events::BytesStart,
) -> Result<PushPortScheduleFormations> {
	let rid = str_attr(open, b"rid")?.ok_or_else(|| anyhow!("<scheduleFormations> missing 'rid'"))?;

	let mut formations: Vec<PushPortFormation> = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"formation" => {
				let formation_id = str_attr(element, b"fid")?.unwrap_or_default();

				buffer.clear();

				let coaches = parse_formation_coaches(reader)?;

				formations.push(PushPortFormation {
					formation_id,
					coaches,
				});
			}

			Event::End(ref element) if local_name(&element.name()) == b"scheduleFormations" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortScheduleFormations {
		darwin_id: rid,
		formations,
	})
}

fn parse_formation_coaches(reader: &mut Reader<&[u8]>) -> Result<Vec<PushPortFormationCoach>> {
	let mut coaches: Vec<PushPortFormationCoach> = Vec::new();
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"coach" => {
				let coach_number = str_attr(element, b"coachNumber")?.unwrap_or_default();
				let coach_class = str_attr(element, b"coachClass")?;

				buffer.clear();

				let (toilet_status, toilet_type) = parse_formation_coach_toilet(reader)?;

				coaches.push(PushPortFormationCoach {
					coach_number,
					coach_class,
					toilet_status,
					toilet_type,
				});
			}

			Event::Empty(ref element) if local_name(&element.name()) == b"coach" => {
				coaches.push(PushPortFormationCoach {
					coach_number: str_attr(element, b"coachNumber")?.unwrap_or_default(),
					coach_class: str_attr(element, b"coachClass")?,
					toilet_status: None,
					toilet_type: None,
				});
			}

			Event::End(ref element) if local_name(&element.name()) == b"coaches" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(coaches)
}

fn parse_formation_coach_toilet(reader: &mut Reader<&[u8]>) -> Result<(Option<String>, Option<String>)> {
	let mut toilet_status: Option<String> = None;
	let mut toilet_type: Option<String> = None;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(ref element) if local_name(&element.name()) == b"toilet" => {
				toilet_status = str_attr(element, b"status")?;

				buffer.clear();

				toilet_type = Some(collect_inner_text(reader, b"toilet")?);
			}

			Event::Empty(ref element) if local_name(&element.name()) == b"toilet" => {
				toilet_status = str_attr(element, b"status")?;
			}

			Event::End(ref element) if local_name(&element.name()) == b"coach" => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok((toilet_status, toilet_type))
}

fn parse_disruption_reason(reader: &mut Reader<&[u8]>, end_tag: &[u8]) -> Result<PushPortDisruptionReason> {
	let mut code: i16 = 0;
	let mut tiploc: Option<String> = None;
	let mut is_near = false;

	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Text(ref text) => {
				let s = text.decode()?;
				code = s.trim().parse().unwrap_or(0);
			}

			Event::End(ref element) if local_name(&element.name()) == end_tag => break,
			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(PushPortDisruptionReason {
		code,
		tiploc,
		is_near,
	})
}

fn collect_inner_text(reader: &mut Reader<&[u8]>, end_tag: &[u8]) -> Result<String> {
	let mut output = String::new();
	let mut buffer = Vec::new();
	let mut depth: usize = 0;

	loop {
		match reader.read_event_into(&mut buffer)? {
			// Msg bodies wrap their text in <p>/<a> etc - capture text at any depth, not just
			// directly inside the end tag, or nested content (most of it) is silently dropped.
			Event::Text(ref text) => {
				output.push_str(&text.decode()?);
			}

			Event::Start(_) => depth += 1,

			Event::End(ref element) => {
				if depth == 0 && local_name(&element.name()) == end_tag {
					break;
				}

				depth = depth.saturating_sub(1);
			}

			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	Ok(output.trim().to_string())
}

fn skip_element(reader: &mut Reader<&[u8]>, tag: &[u8]) -> Result<()> {
	let mut depth: usize = 1;
	let mut buffer = Vec::new();

	loop {
		match reader.read_event_into(&mut buffer)? {
			Event::Start(_) => depth += 1,

			Event::End(ref element) => {
				depth -= 1;
				if depth == 0 && local_name(&element.name()) == tag {
					break;
				}
			}

			Event::Eof => break,

			_ => {}
		}

		buffer.clear();
	}

	eprintln!("Skipped XML element: {:?}", String::from_utf8_lossy(tag));

	Ok(())
}

fn local_name<'a>(name: &'a quick_xml::name::QName<'a>) -> &'a [u8] {
	let bytes = name.as_ref();
	if let Some(pos) = bytes.iter().position(|&b| b == b':') {
		&bytes[pos + 1..]
	} else {
		bytes
	}
}

fn str_attr(element: &quick_xml::events::BytesStart, name: &[u8]) -> Result<Option<String>> {
	for attr in element.attributes().flatten() {
		if local_name(&attr.key.into()) == name {
			return Ok(Some(attr.unescape_value()?.into_owned()));
		}
	}

	Ok(None)
}

fn bool_attr(element: &quick_xml::events::BytesStart, name: &[u8]) -> bool {
	str_attr(element, name).ok().flatten().as_deref() == Some("true")
}
