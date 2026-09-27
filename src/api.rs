use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::metrics::{self, Metrics};
use crate::redis::Redis;

#[derive(Clone)]
pub struct AppState {
	pub redis: Arc<Redis>,
	pub metrics: Arc<Metrics>,
}

fn router(state: AppState) -> Router {
	Router::new()
		.route("/", get(get_status))
		.route("/journeys", get(list_journeys))
		.route("/journeys/{rid}", get(get_journey))
		.route("/journeys/{rid}/locations", get(get_journey_locations))
		.route("/journeys/{rid}/formations", get(get_journey_formations))
		.route("/journeys/{rid}/formation/{formation_id}", get(get_journey_formation))
		.route("/journeys/{rid}/loading", get(get_journey_loading))
		.route("/journeys/{rid}/associations", get(get_journey_associations))
		.route("/stations/{tiploc}/board", get(get_station_board))
		.route("/stations/{crs}/messages", get(get_station_messages))
		.route("/messages/{id}", get(get_message))
		.route("/timetable", get(get_timetable))
		.with_state(state)
}

pub async fn serve(state: AppState, addr: SocketAddr) -> Result<()> {
	let app = router(state);

	let listener = TcpListener::bind(addr).await.context(format!("Unable to bind HTTP listener to '{addr}'"))?;
	println!("HTTP API listening on: http://{}", listener.local_addr()?);

	axum::serve(listener, app).await.context("HTTP server error")
}

struct APIError {
	status: StatusCode,
	message: String,
}

impl APIError {
	fn internal(message: impl Into<String>) -> Self {
		Self {
			status: StatusCode::INTERNAL_SERVER_ERROR,
			message: message.into(),
		}
	}

	fn not_found(message: impl Into<String>) -> Self {
		Self {
			status: StatusCode::NOT_FOUND,
			message: message.into(),
		}
	}
}

impl IntoResponse for APIError {
	fn into_response(self) -> Response {
		(self.status, Json(json!({ "error": self.message }))).into_response()
	}
}

// Redis access is blocking - never call it directly on an async handler's task.
async fn blocking<T, F>(redis: Arc<Redis>, f: F) -> Result<T, APIError>
where
	F: FnOnce(&Redis) -> Result<T> + Send + 'static,
	T: Send + 'static,
{
	match tokio::task::spawn_blocking(move || f(&redis)).await {
		Ok(Ok(value)) => Ok(value),
		Ok(Err(error)) => Err(APIError::internal(error.to_string())),
		Err(join_error) => Err(APIError::internal(join_error.to_string())),
	}
}

// GET /
async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
	let redis = state.redis.clone();

	let latency = tokio::task::spawn_blocking({
		let redis = redis.clone();
		move || redis.ping()
	})
	.await
	.unwrap_or(None);
	let redis_healthy = latency.is_some();

	let keys = if redis_healthy {
		tokio::task::spawn_blocking({
			let redis = redis.clone();
			move || redis.key_count()
		})
		.await
		.unwrap_or(None)
	} else {
		None
	};

	let stomp_healthy = state.metrics.stomp_connected();
	let healthy = redis_healthy && stomp_healthy;

	let body = json!({
		"process": {
			"uptime": state.metrics.uptime_seconds(),
			"memory": metrics::process_memory_kib(),
		},
		"darwin": {
			"stomp": {
				"healthy": stomp_healthy,
				"connections": state.metrics.stomp_subscription_count(),
			},
			"drift": state.metrics.stomp_drift_ms(),
			"events": state.metrics.event_counts_json(),
		},
		"redis": {
			"healthy": redis_healthy,
			"connections": if redis_healthy { 1 } else { 0 },
			"latency": latency,
			"keys": keys,
			"drift": state.metrics.redis_drift_ms(),
		},
	});

	let status = if healthy {
		StatusCode::OK
	} else {
		StatusCode::SERVICE_UNAVAILABLE
	};
	(status, Json(body))
}

#[derive(Deserialize, Default)]
struct JourneysQuery {
	schedule_uid: Option<String>,
	operator_code: Option<String>,
	tiploc: Option<String>,
	headcode: Option<String>,
}

async fn list_journeys(
	State(state): State<AppState>,
	Query(query): Query<JourneysQuery>,
) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let rids: Vec<String> = if let Some(schedule_uid) = query.schedule_uid {
		let key = redis.key(&["index", "journeys_by_schedule_uid", &schedule_uid]);
		blocking(redis.clone(), move |redis| redis.smembers(&key)).await?
	} else if let Some(operator_code) = query.operator_code {
		let key = redis.key(&["index", "journeys_by_operator", &operator_code]);
		blocking(redis.clone(), move |redis| redis.smembers(&key)).await?
	} else if let Some(tiploc) = query.tiploc {
		let key = redis.key(&["index", "journeys_by_tiploc", &tiploc]);
		blocking(redis.clone(), move |redis| redis.zrevrange_all(&key)).await?
	} else if let Some(headcode) = query.headcode {
		let key = redis.key(&["index", "journeys_by_headcode", &headcode]);
		blocking(redis.clone(), move |redis| redis.smembers(&key)).await?
	} else {
		let key = redis.key(&["index", "active_journeys"]);
		blocking(redis.clone(), move |redis| redis.zrevrange_all(&key)).await?
	};

	let mut journeys = Vec::with_capacity(rids.len());
	for rid in rids {
		let key = redis.key(&["journey", &rid]);
		let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;

		if !hash.is_empty() {
			journeys.push(journey_json(&rid, &hash));
		}
	}

	Ok(Json(json!({ "journeys": journeys, "count": journeys.len() })))
}

async fn get_journey(State(state): State<AppState>, Path(rid): Path<String>) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let journey_key = redis.key(&["journey", &rid]);
	let hash = blocking(redis.clone(), move |redis| redis.hash(&journey_key)).await?;

	if hash.is_empty() {
		return Err(APIError::not_found("journey not found"));
	}

	let locations = fetch_locations(&redis, &rid, has_schedule(&hash)).await?;
	let formation_ids = fetch_formation_ids(&redis, &rid).await?;
	let association_refs = fetch_association_refs(&redis, &rid).await?;

	let mut body = journey_json(&rid, &hash);
	if let Value::Object(ref mut map) = body {
		map.insert("locations".to_string(), Value::Array(locations));
		map.insert("formations".to_string(), Value::Array(formation_ids.into_iter().map(Value::String).collect()));
		map.insert("associations".to_string(), Value::Array(association_refs.into_iter().map(Value::String).collect()));
	}

	Ok(Json(body))
}

async fn get_journey_locations(
	State(state): State<AppState>,
	Path(rid): Path<String>,
) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let journey_key = redis.key(&["journey", &rid]);
	let journey_hash = blocking(redis.clone(), move |redis| redis.hash(&journey_key)).await?;

	if journey_hash.is_empty() {
		return Err(APIError::not_found("journey not found"));
	}

	let locations = fetch_locations(&redis, &rid, has_schedule(&journey_hash)).await?;

	Ok(Json(json!({ "rid": rid, "locations": locations })))
}

async fn get_journey_formations(
	State(state): State<AppState>,
	Path(rid): Path<String>,
) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();
	let formation_ids = fetch_formation_ids(&redis, &rid).await?;

	let mut formations = Vec::with_capacity(formation_ids.len());
	for formation_id in formation_ids {
		let key = redis.key(&["journey", &rid, "formation", &formation_id]);
		let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;

		if !hash.is_empty() {
			formations.push(formation_json(&formation_id, &hash));
		}
	}

	Ok(Json(json!({ "rid": rid, "formations": formations })))
}

async fn get_journey_formation(
	State(state): State<AppState>,
	Path((rid, formation_id)): Path<(String, String)>,
) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let key = redis.key(&["journey", &rid, "formation", &formation_id]);
	let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;

	if hash.is_empty() {
		return Err(APIError::not_found("formation not found"));
	}

	Ok(Json(formation_json(&formation_id, &hash)))
}

async fn get_journey_loading(State(state): State<AppState>, Path(rid): Path<String>) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let locations_key = redis.key(&["journey", &rid, "loading_locations"]);
	let location_ids = blocking(redis.clone(), move |redis| redis.smembers(&locations_key)).await?;

	let mut loading = Vec::with_capacity(location_ids.len());
	for location_id in location_ids {
		let key = redis.key(&["journey", &rid, "loading", &location_id]);
		let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;

		if !hash.is_empty() {
			loading.push(loading_json(&location_id, &hash));
		}
	}

	Ok(Json(json!({ "rid": rid, "loading": loading })))
}

async fn get_journey_associations(
	State(state): State<AppState>,
	Path(rid): Path<String>,
) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();
	let refs = fetch_association_refs(&redis, &rid).await?;

	let mut associations = Vec::with_capacity(refs.len());
	for reference in refs {
		let parts: Vec<&str> = reference.splitn(3, ':').collect();
		let &[tiploc, main_rid, associated_rid] = parts.as_slice() else {
			continue;
		};

		let key = redis.key(&["association", tiploc, main_rid, associated_rid]);
		let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;

		if !hash.is_empty() {
			associations.push(association_json(&reference, &hash));
		}
	}

	Ok(Json(json!({ "rid": rid, "associations": associations })))
}

// `has_schedule` - true once a <schedule> has been seen for this rid, giving location_order its
// real sequence index (trusted as-is: schedule order is authoritative, even across a midnight
// rollover where clock time alone would sort wrong). False for a TS-only rid, where location_order
// only has first-seen wall-clock scores - re-sort those by each location's own working time instead.
async fn fetch_locations(redis: &Arc<Redis>, rid: &str, has_schedule: bool) -> Result<Vec<Value>, APIError> {
	let order_key = redis.key(&["journey", rid, "location_order"]);
	let mut location_ids = blocking(redis.clone(), move |redis| redis.zrange_all(&order_key)).await?;

	if !has_schedule {
		location_ids.sort_by(|a, b| location_sort_key(a).cmp(&location_sort_key(b)));
	}

	let mut locations = Vec::with_capacity(location_ids.len());
	for location_id in location_ids {
		let key = redis.key(&["journey", rid, "location", &location_id]);
		let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;
		locations.push(location_json(&location_id, &hash));
	}

	Ok(locations)
}

// Earliest of arrival/departure/pass, "XXXXXX" (sorts last) if a location has none set.
fn location_sort_key(location_id: &str) -> String {
	let (_, arrival, departure, pass) = parse_location_id(location_id);

	arrival.or(departure).or(pass).unwrap_or_else(|| "XXXXXX".to_string())
}

// A schedule (not just forecasts) has been seen for this rid iff "category" is set - only
// store_schedule ever writes it.
fn has_schedule(hash: &HashMap<String, String>) -> bool {
	hash.contains_key("category")
}

async fn fetch_formation_ids(redis: &Arc<Redis>, rid: &str) -> Result<Vec<String>, APIError> {
	let key = redis.key(&["journey", rid, "formations"]);
	blocking(redis.clone(), move |redis| redis.smembers(&key)).await
}

async fn fetch_association_refs(redis: &Arc<Redis>, rid: &str) -> Result<Vec<String>, APIError> {
	let key = redis.key(&["journey", rid, "associations"]);
	blocking(redis.clone(), move |redis| redis.smembers(&key)).await
}

async fn get_station_board(State(state): State<AppState>, Path(tiploc): Path<String>) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let index_key = redis.key(&["index", "journeys_by_tiploc", &tiploc]);
	let rids = blocking(redis.clone(), move |redis| redis.zrevrange_all(&index_key)).await?;

	let mut services = Vec::new();
	let prefix = format!("{tiploc}_");

	for rid in rids {
		let journey_key = redis.key(&["journey", &rid]);
		let journey_hash = blocking(redis.clone(), move |redis| redis.hash(&journey_key)).await?;

		if journey_hash.is_empty() {
			continue;
		}

		let order_key = redis.key(&["journey", &rid, "location_order"]);
		let location_ids = blocking(redis.clone(), move |redis| redis.zrange_all(&order_key)).await?;

		for location_id in location_ids.into_iter().filter(|id| id.starts_with(&prefix)) {
			let location_key = redis.key(&["journey", &rid, "location", &location_id]);
			let location_hash = blocking(redis.clone(), move |redis| redis.hash(&location_key)).await?;

			services.push(json!({
				"rid": rid,
				"headcode": hash_str(&journey_hash, "headcode"),
				"operator_code": hash_str(&journey_hash, "operator_code"),
				"is_deactivated": hash_bool(&journey_hash, "is_deactivated"),
				"location": location_json(&location_id, &location_hash),
			}));
		}
	}

	Ok(Json(json!({ "tiploc": tiploc, "services": services })))
}

async fn get_station_messages(State(state): State<AppState>, Path(crs): Path<String>) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let index_key = redis.key(&["index", "station_messages_by_station", &crs]);
	let ids = blocking(redis.clone(), move |redis| redis.smembers(&index_key)).await?;

	let mut messages = Vec::with_capacity(ids.len());
	for id in ids {
		let message_key = redis.key(&["station_message", &id]);
		let hash = blocking(redis.clone(), move |redis| redis.hash(&message_key)).await?;

		if hash.is_empty() {
			continue;
		}

		let stations_key = redis.key(&["station_message", &id, "stations"]);
		let stations = blocking(redis.clone(), move |redis| redis.smembers(&stations_key)).await?;

		messages.push(message_json(&id, &hash, stations));
	}

	Ok(Json(json!({ "crs": crs, "messages": messages })))
}

async fn get_message(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let message_key = redis.key(&["station_message", &id]);
	let hash = blocking(redis.clone(), move |redis| redis.hash(&message_key)).await?;

	if hash.is_empty() {
		return Err(APIError::not_found("message not found"));
	}

	let stations_key = redis.key(&["station_message", &id, "stations"]);
	let stations = blocking(redis.clone(), move |redis| redis.smembers(&stations_key)).await?;

	Ok(Json(message_json(&id, &hash, stations)))
}

async fn get_timetable(State(state): State<AppState>) -> Result<Json<Value>, APIError> {
	let redis = state.redis.clone();

	let key = redis.key(&["latest_timetable_reference"]);
	let hash = blocking(redis.clone(), move |redis| redis.hash(&key)).await?;

	if hash.is_empty() {
		return Err(APIError::not_found("no timetable reference available yet"));
	}

	Ok(Json(json!({
		"id": hash_str(&hash, "id"),
		"timetable_file": hash_str(&hash, "timetable_file"),
		"reference_file": hash_str(&hash, "reference_file"),
		"updated_at": hash_num::<i64>(&hash, "updated_at"),
	})))
}

fn hash_str(hash: &HashMap<String, String>, key: &str) -> Option<String> {
	hash.get(key).cloned()
}

fn hash_bool(hash: &HashMap<String, String>, key: &str) -> bool {
	hash.get(key).map(|value| value == "1").unwrap_or(false)
}

fn hash_num<T: std::str::FromStr>(hash: &HashMap<String, String>, key: &str) -> Option<T> {
	hash.get(key).and_then(|value| value.parse().ok())
}

fn parse_location_id(location_id: &str) -> (String, Option<String>, Option<String>, Option<String>) {
	let parts: Vec<&str> = location_id.rsplitn(4, '_').collect();

	let format_time = |raw: &str| -> Option<String> {
		if raw.len() != 6 || raw == "XXXXXX" {
			return None;
		}

		Some(format!("{}:{}:{}", &raw[0..2], &raw[2..4], &raw[4..6]))
	};

	let pass_time = parts.first().and_then(|raw| format_time(raw));
	let working_departure_time = parts.get(1).and_then(|raw| format_time(raw));
	let working_arrival_time = parts.get(2).and_then(|raw| format_time(raw));
	let tiploc = parts.get(3).map(|value| value.to_string()).unwrap_or_else(|| location_id.to_string());

	(tiploc, working_arrival_time, working_departure_time, pass_time)
}

fn journey_json(rid: &str, hash: &HashMap<String, String>) -> Value {
	json!({
		"rid": rid,
		"schedule_uid": hash_str(hash, "schedule_uid"),
		"schedule_start_date": hash_str(hash, "schedule_start_date"),
		"headcode": hash_str(hash, "headcode"),
		"operator_code": hash_str(hash, "operator_code"),
		"category": hash_str(hash, "category"),
		"status": hash_str(hash, "status"),
		"is_active": hash_bool(hash, "is_active"),
		"is_passenger": hash_bool(hash, "is_passenger"),
		"is_charter": hash_bool(hash, "is_charter"),
		"is_deleted": hash_bool(hash, "is_deleted"),
		"is_reverse_formation": hash_bool(hash, "is_reverse_formation"),
		"is_deactivated": hash_bool(hash, "is_deactivated"),
		"cancel_reason_code": hash_num::<i16>(hash, "cancel_reason_code"),
		"delay_reason_code": hash_num::<i16>(hash, "delay_reason_code"),
		"last_updated_at": hash_num::<i64>(hash, "last_updated_at"),
	})
}

fn location_json(location_id: &str, hash: &HashMap<String, String>) -> Value {
	let (tiploc, working_arrival_time, working_departure_time, pass_time) = parse_location_id(location_id);

	let platform = hash.contains_key("platform_value").then(|| {
		json!({
			"value": hash_str(hash, "platform_value"),
			"source": hash_str(hash, "platform_source"),
			"is_suppressed": hash_bool(hash, "platform_is_suppressed"),
			"is_confirmed": hash_bool(hash, "platform_is_confirmed"),
			"is_cis_suppressed": hash_bool(hash, "platform_is_cis_suppressed"),
		})
	});

	json!({
		"location_id": location_id,
		"tiploc": tiploc,
		"kind": hash_str(hash, "kind"),
		"working_arrival_time": working_arrival_time,
		"working_departure_time": working_departure_time,
		"pass_time": pass_time,
		"public_arrival_time": hash_str(hash, "public_arrival_time"),
		"public_departure_time": hash_str(hash, "public_departure_time"),
		"planned_platform": hash_str(hash, "platform"),
		"platform": platform,
		"flags": hash_str(hash, "flags"),
		"original_flags": hash_str(hash, "original_flags"),
		"is_cancelled": hash_bool(hash, "is_cancelled"),
		"false_destination": hash_str(hash, "false_destination"),
		"reroute_delay": hash_num::<i16>(hash, "reroute_delay"),
		"arrival": time_data_json(hash, "arrival"),
		"departure": time_data_json(hash, "departure"),
		"pass": time_data_json(hash, "pass"),
		"length_in_coaches": hash_num::<u16>(hash, "length_in_coaches"),
		"is_suppressed": hash_bool(hash, "is_suppressed"),
		"detach_front": hash_bool(hash, "detach_front"),
	})
}

fn time_data_json(hash: &HashMap<String, String>, prefix: &str) -> Option<Value> {
	let estimated = hash_str(hash, &format!("{prefix}_estimated"));
	let actual = hash_str(hash, &format!("{prefix}_actual"));
	let working_estimated = hash_str(hash, &format!("{prefix}_working_estimated"));
	let minimum_estimated = hash_str(hash, &format!("{prefix}_minimum_estimated"));
	let source_system = hash_str(hash, &format!("{prefix}_source_system"));
	let source_cis_code = hash_str(hash, &format!("{prefix}_source_cis_code"));
	let is_delayed = hash_bool(hash, &format!("{prefix}_is_delayed"));
	let is_actual_removed = hash_bool(hash, &format!("{prefix}_is_actual_removed"));
	let is_estimate_unknown = hash_bool(hash, &format!("{prefix}_is_estimate_unknown"));

	let has_any = estimated.is_some()
		|| actual.is_some()
		|| working_estimated.is_some()
		|| minimum_estimated.is_some()
		|| source_system.is_some()
		|| source_cis_code.is_some()
		|| is_delayed
		|| is_actual_removed
		|| is_estimate_unknown;

	if !has_any {
		return None;
	}

	Some(json!({
		"estimated": estimated,
		"actual": actual,
		"working_estimated": working_estimated,
		"minimum_estimated": minimum_estimated,
		"is_delayed": is_delayed,
		"is_actual_removed": is_actual_removed,
		"is_estimate_unknown": is_estimate_unknown,
		"source_system": source_system,
		"source_cis_code": source_cis_code,
	}))
}

fn coach_numbers(hash: &HashMap<String, String>, suffixes: &[&str]) -> Vec<String> {
	let mut numbers: Vec<String> = Vec::new();

	for key in hash.keys() {
		let Some(rest) = key.strip_prefix("coach_") else {
			continue;
		};

		for suffix in suffixes {
			if let Some(number) = rest.strip_suffix(suffix) {
				if !numbers.iter().any(|existing| existing == number) {
					numbers.push(number.to_string());
				}
				break;
			}
		}
	}

	numbers.sort();
	numbers
}

fn formation_json(formation_id: &str, hash: &HashMap<String, String>) -> Value {
	let numbers = coach_numbers(hash, &["_class", "_toilet_status", "_toilet_type"]);

	let coaches: Vec<Value> = numbers
		.iter()
		.map(|number| {
			json!({
				"coach_number": number,
				"class": hash_str(hash, &format!("coach_{number}_class")),
				"toilet_status": hash_str(hash, &format!("coach_{number}_toilet_status")),
				"toilet_type": hash_str(hash, &format!("coach_{number}_toilet_type")),
			})
		})
		.collect();

	json!({ "formation_id": formation_id, "coaches": coaches })
}

fn loading_json(location_id: &str, hash: &HashMap<String, String>) -> Value {
	let numbers = coach_numbers(hash, &["_percentage", "_source", "_source_system"]);

	let coaches: Vec<Value> = numbers
		.iter()
		.map(|number| {
			json!({
				"coach_number": number,
				"percentage": hash_num::<u8>(hash, &format!("coach_{number}_percentage")),
				"source": hash_str(hash, &format!("coach_{number}_source")),
				"source_system": hash_str(hash, &format!("coach_{number}_source_system")),
			})
		})
		.collect();

	json!({
		"location_id": location_id,
		"formation_id": hash_str(hash, "formation_id"),
		"coaches": coaches,
	})
}

fn association_json(reference: &str, hash: &HashMap<String, String>) -> Value {
	let mut parts = reference.splitn(3, ':');
	let tiploc = parts.next().unwrap_or_default();
	let main_rid = parts.next().unwrap_or_default();
	let associated_rid = parts.next().unwrap_or_default();

	json!({
		"tiploc": tiploc,
		"main_rid": main_rid,
		"associated_rid": associated_rid,
		"category": hash_str(hash, "category"),
		"is_cancelled": hash_bool(hash, "is_cancelled"),
		"is_deleted": hash_bool(hash, "is_deleted"),
		"main_working_arrival_time": hash_str(hash, "main_working_arrival_time"),
		"main_working_departure_time": hash_str(hash, "main_working_departure_time"),
		"main_public_arrival_time": hash_str(hash, "main_public_arrival_time"),
		"main_public_departure_time": hash_str(hash, "main_public_departure_time"),
		"main_pass_time": hash_str(hash, "main_pass_time"),
		"associated_working_arrival_time": hash_str(hash, "associated_working_arrival_time"),
		"associated_working_departure_time": hash_str(hash, "associated_working_departure_time"),
		"associated_public_arrival_time": hash_str(hash, "associated_public_arrival_time"),
		"associated_public_departure_time": hash_str(hash, "associated_public_departure_time"),
		"associated_pass_time": hash_str(hash, "associated_pass_time"),
	})
}

fn message_json(id: &str, hash: &HashMap<String, String>, stations: Vec<String>) -> Value {
	json!({
		"id": id.parse::<i64>().ok(),
		"category": hash_str(hash, "category"),
		"severity": hash_num::<u8>(hash, "severity"),
		"is_suppressed": hash_bool(hash, "is_suppressed"),
		"message_text": hash_str(hash, "message_text"),
		"stations": stations,
	})
}
