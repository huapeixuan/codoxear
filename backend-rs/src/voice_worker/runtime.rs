use std::time::{Duration, SystemTime};

pub trait VoiceClock: Send + Sync {
    fn now(&self) -> SystemTime;
    fn monotonic_sleep_duration(&self, seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds.max(0.0))
    }
}

#[derive(Debug, Clone, Default)]
pub struct SystemVoiceClock;

impl VoiceClock for SystemVoiceClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoiceRuntimeSnapshot {
    pub queue_depth: usize,
    pub active_listener_count: usize,
    pub segment_count: usize,
    pub media_sequence: usize,
    pub last_error: String,
}
