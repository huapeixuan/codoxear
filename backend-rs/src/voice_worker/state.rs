use crate::state_files::write_value;
use crate::voice_state::now_seconds;
use crate::voice_worker::hls::{empty_playlist, HLS_MAX_SEGMENTS};
use crate::voice_worker::ledger::DELIVERY_LEDGER_FILE;
use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub const LISTENER_TTL_SECONDS: f64 = 45.0;

static RUNTIMES: OnceLock<Mutex<HashMap<String, Arc<VoiceRuntime>>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenerUpdate {
    pub active_listener_count: usize,
    pub last_listener_dropped: bool,
}

#[derive(Debug)]
pub struct VoiceRuntime {
    app_dir: PathBuf,
    state: Mutex<VoiceRuntimeInner>,
}

#[derive(Debug, Default)]
struct VoiceRuntimeInner {
    listeners: HashMap<String, f64>,
    listener_epoch: u64,
    queue: VecDeque<QueuedVoiceTask>,
    generating: Option<QueuedVoiceTask>,
    prepared: Option<QueuedVoiceTask>,
    playing: Option<QueuedVoiceTask>,
    segment_count: usize,
    media_sequence: usize,
    last_error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedVoiceTask {
    pub message_id: String,
    pub source_message_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceRuntimeSnapshot {
    pub queue_depth: usize,
    pub active_listener_count: usize,
    pub segment_count: usize,
    pub media_sequence: usize,
    pub last_error: String,
}

pub fn runtime_for_app_dir(app_dir: &Path) -> Arc<VoiceRuntime> {
    let key = app_dir.to_string_lossy().into_owned();
    let registry = RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry.lock().expect("voice runtime registry lock");
    registry
        .entry(key)
        .or_insert_with(|| Arc::new(VoiceRuntime::new(app_dir.to_path_buf())))
        .clone()
}

pub fn reset_runtime_registry_for_tests() {
    if let Some(registry) = RUNTIMES.get() {
        registry.lock().unwrap().clear();
    }
}

impl VoiceRuntime {
    pub fn new(app_dir: PathBuf) -> Self {
        Self {
            app_dir,
            state: Mutex::new(VoiceRuntimeInner {
                media_sequence: 1,
                ..VoiceRuntimeInner::default()
            }),
        }
    }

    pub fn listener_heartbeat(
        &self,
        client_id: &str,
        enabled: bool,
        now_ts: f64,
    ) -> Result<ListenerUpdate, String> {
        let cid = client_id.trim();
        if cid.is_empty() {
            return Err("client_id required".to_string());
        }
        let (dropped, update) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "voice runtime lock poisoned")?;
            prune_listeners(&mut state.listeners, now_ts);
            let previous_count = state.listeners.len();
            if enabled {
                state.listeners.insert(cid.to_string(), now_ts);
            } else {
                state.listeners.remove(cid);
            }
            prune_listeners(&mut state.listeners, now_ts);
            let count = state.listeners.len();
            if previous_count > 0 && count == 0 {
                state.listener_epoch = state.listener_epoch.saturating_add(1);
                let mut dropped = Vec::new();
                dropped.extend(state.queue.drain(..));
                dropped.extend(state.generating.take());
                dropped.extend(state.prepared.take());
                state.playing = None;
                state.segment_count = 0;
                state.media_sequence = 1;
                state.last_error.clear();
                (
                    dropped,
                    ListenerUpdate {
                        active_listener_count: count,
                        last_listener_dropped: true,
                    },
                )
            } else {
                (
                    Vec::new(),
                    ListenerUpdate {
                        active_listener_count: count,
                        last_listener_dropped: false,
                    },
                )
            }
        };
        if update.last_listener_dropped {
            self.reset_hls_playlist()?;
        }
        if !dropped.is_empty() {
            self.mark_tasks_skipped_no_listener(&dropped)?;
        }
        Ok(update)
    }

    pub fn snapshot(&self, now_ts: f64) -> VoiceRuntimeSnapshot {
        let mut state = self.state.lock().expect("voice runtime lock");
        prune_listeners(&mut state.listeners, now_ts);
        VoiceRuntimeSnapshot {
            queue_depth: state.queue.len(),
            active_listener_count: state.listeners.len(),
            segment_count: state.segment_count,
            media_sequence: state.media_sequence.max(1),
            last_error: state.last_error.clone(),
        }
    }

    pub fn enqueue_for_tests(&self, task: QueuedVoiceTask) {
        self.state.lock().unwrap().queue.push_back(task);
    }

    pub fn set_last_error(&self, message: &str) {
        self.state.lock().unwrap().last_error = message.trim().to_string();
    }

    pub fn set_hls_snapshot_for_tests(&self, segment_count: usize, media_sequence: usize) {
        let mut state = self.state.lock().unwrap();
        state.segment_count = segment_count.min(HLS_MAX_SEGMENTS);
        state.media_sequence = media_sequence.max(1);
    }

    fn reset_hls_playlist(&self) -> Result<(), String> {
        let audio_dir = self.app_dir.join("audio");
        let segments_dir = audio_dir.join("segments");
        fs::create_dir_all(&segments_dir)
            .map_err(|err| format!("create {}: {err}", segments_dir.display()))?;
        if let Ok(entries) = fs::read_dir(&segments_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) == Some("ts") {
                    let _ = fs::remove_file(path);
                }
            }
        }
        fs::write(audio_dir.join("live.m3u8"), empty_playlist(1))
            .map_err(|err| format!("write live.m3u8: {err}"))?;
        Ok(())
    }

    fn mark_tasks_skipped_no_listener(&self, tasks: &[QueuedVoiceTask]) -> Result<(), String> {
        let ids = tasks
            .iter()
            .flat_map(|task| {
                task.source_message_ids
                    .iter()
                    .chain(std::iter::once(&task.message_id))
            })
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if ids.is_empty() {
            return Ok(());
        }
        let path = self.app_dir.join(DELIVERY_LEDGER_FILE);
        let mut ledger = read_ledger_object(&path);
        let now = now_seconds();
        let mut dirty = false;
        for id in ids {
            let Some(row) = ledger.get_mut(id).and_then(Value::as_object_mut) else {
                continue;
            };
            row.insert("narrated_status".to_string(), json!("skipped"));
            if row.get("summary_status").and_then(Value::as_str) == Some("pending") {
                row.insert("summary_status".to_string(), json!("skipped"));
            }
            row.insert("last_error".to_string(), json!("no active listener"));
            row.insert("updated_ts".to_string(), json!(now));
            dirty = true;
        }
        if dirty {
            write_value(&path, &Value::Object(ledger)).map_err(|(_, message)| message)?;
        }
        Ok(())
    }
}

fn prune_listeners(listeners: &mut HashMap<String, f64>, now_ts: f64) {
    listeners.retain(|_, seen_at| (now_ts - *seen_at) <= LISTENER_TTL_SECONDS);
}

fn read_ledger_object(path: &Path) -> Map<String, Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

pub fn update_subscription_success_failure(
    app_dir: &Path,
    target_id: &str,
    success: bool,
    error: &str,
    now_ts: f64,
) -> Result<(), (StatusCode, String)> {
    let path = app_dir.join("push_subscriptions.json");
    let mut items = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    for item in &mut items {
        let Some(record) = item.as_object_mut() else {
            continue;
        };
        if record.get("id").and_then(Value::as_str) != Some(target_id) {
            continue;
        }
        record.insert("updated_ts".to_string(), json!(now_ts));
        if success {
            record.insert("last_success_ts".to_string(), json!(now_ts));
            record.insert("last_error".to_string(), json!(""));
        } else {
            record.insert("last_failure_ts".to_string(), json!(now_ts));
            record.insert("last_error".to_string(), json!(clip_error(error)));
        }
    }
    write_value(&path, &Value::Array(items))
}

fn clip_error(raw: &str) -> String {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 400 {
        return text;
    }
    let mut out = text.chars().take(399).collect::<String>();
    out.push_str("...");
    out
}
