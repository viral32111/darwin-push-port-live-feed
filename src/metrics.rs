use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::xml::PushPortEvent;

pub struct Metrics {
	started_at: Instant,

	train_status: AtomicU64,
	schedule: AtomicU64,
	deactivated: AtomicU64,
	association: AtomicU64,
	station_message: AtomicU64,
	train_alert: AtomicU64,
	train_order: AtomicU64,
	tracking_id: AtomicU64,
	alarm: AtomicU64,
	timetable_id: AtomicU64,
	formation_loading: AtomicU64,
	schedule_formations: AtomicU64,

	stomp_connected: AtomicBool,
	stomp_subscription_count: AtomicU64,

	last_stomp_message_at_ms: AtomicI64, // 0 = never received
	last_redis_write_at_ms: AtomicI64,   // 0 = never written
}

impl Metrics {
	pub fn new() -> Self {
		Self {
			started_at: Instant::now(),

			train_status: AtomicU64::new(0),
			schedule: AtomicU64::new(0),
			deactivated: AtomicU64::new(0),
			association: AtomicU64::new(0),
			station_message: AtomicU64::new(0),
			train_alert: AtomicU64::new(0),
			train_order: AtomicU64::new(0),
			tracking_id: AtomicU64::new(0),
			alarm: AtomicU64::new(0),
			timetable_id: AtomicU64::new(0),
			formation_loading: AtomicU64::new(0),
			schedule_formations: AtomicU64::new(0),

			stomp_connected: AtomicBool::new(false),
			stomp_subscription_count: AtomicU64::new(0),

			last_stomp_message_at_ms: AtomicI64::new(0),
			last_redis_write_at_ms: AtomicI64::new(0),
		}
	}

	pub fn record_event(&self, event: &PushPortEvent) {
		let counter = match event {
			PushPortEvent::TrainStatus(_) => &self.train_status,
			PushPortEvent::Schedule(_) => &self.schedule,
			PushPortEvent::Deactivated {
				..
			} => &self.deactivated,
			PushPortEvent::Association(_) => &self.association,
			PushPortEvent::StationMessage(_) => &self.station_message,
			PushPortEvent::TrainAlert(_) => &self.train_alert,
			PushPortEvent::TrainOrder(_) => &self.train_order,
			PushPortEvent::TrackingId(_) => &self.tracking_id,
			PushPortEvent::Alarm(_) => &self.alarm,
			PushPortEvent::TimetableId(_) => &self.timetable_id,
			PushPortEvent::FormationLoading(_) => &self.formation_loading,
			PushPortEvent::ScheduleFormations(_) => &self.schedule_formations,
		};

		counter.fetch_add(1, Ordering::Relaxed);
	}

	pub fn record_stomp_message(&self) {
		self.last_stomp_message_at_ms.store(now_ms(), Ordering::Relaxed);
	}

	pub fn record_redis_write(&self) {
		self.last_redis_write_at_ms.store(now_ms(), Ordering::Relaxed);
	}

	pub fn set_stomp_connected(&self, connected: bool, subscription_count: u64) {
		self.stomp_connected.store(connected, Ordering::Relaxed);
		self.stomp_subscription_count.store(subscription_count, Ordering::Relaxed);
	}

	pub fn uptime_seconds(&self) -> u64 {
		self.started_at.elapsed().as_secs()
	}

	pub fn stomp_connected(&self) -> bool {
		self.stomp_connected.load(Ordering::Relaxed)
	}

	pub fn stomp_subscription_count(&self) -> u64 {
		self.stomp_subscription_count.load(Ordering::Relaxed)
	}

	pub fn stomp_drift_ms(&self) -> Option<i64> {
		drift_ms(self.last_stomp_message_at_ms.load(Ordering::Relaxed))
	}

	pub fn redis_drift_ms(&self) -> Option<i64> {
		drift_ms(self.last_redis_write_at_ms.load(Ordering::Relaxed))
	}

	pub fn event_counts_json(&self) -> Value {
		json!({
			"train_status": self.train_status.load(Ordering::Relaxed),
			"schedule": self.schedule.load(Ordering::Relaxed),
			"deactivated": self.deactivated.load(Ordering::Relaxed),
			"association": self.association.load(Ordering::Relaxed),
			"station_message": self.station_message.load(Ordering::Relaxed),
			"train_alert": self.train_alert.load(Ordering::Relaxed),
			"train_order": self.train_order.load(Ordering::Relaxed),
			"tracking_id": self.tracking_id.load(Ordering::Relaxed),
			"alarm": self.alarm.load(Ordering::Relaxed),
			"timetable_id": self.timetable_id.load(Ordering::Relaxed),
			"formation_loading": self.formation_loading.load(Ordering::Relaxed),
			"schedule_formations": self.schedule_formations.load(Ordering::Relaxed),
		})
	}
}

fn drift_ms(last_ms: i64) -> Option<i64> {
	if last_ms == 0 {
		return None;
	}

	Some((now_ms() - last_ms).max(0))
}

fn now_ms() -> i64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_millis() as i64).unwrap_or(0)
}

// Linux-only
pub fn process_memory_kib() -> Option<u64> {
	let status = std::fs::read_to_string("/proc/self/status").ok()?;

	for line in status.lines() {
		if let Some(rest) = line.strip_prefix("VmRSS:") {
			return rest.split_whitespace().next()?.parse().ok();
		}
	}

	None
}
