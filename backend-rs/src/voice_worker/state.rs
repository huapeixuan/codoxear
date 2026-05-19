use crate::state_files::{with_state_file_lock, write_value};
use crate::voice_state::now_seconds;
use crate::voice_worker::delivery::{
    clip_text, collect_task_ids, compact_text, delivery_ledger_path, ledger_sort_ts,
    load_worker_settings, prune_listeners, read_ledger_object, read_mobile_push_targets,
    read_subscription_map, reset_hls_playlist, should_deliver_row, summary_request, tts_request,
    voice_for_session, write_subscription_map,
};
use crate::voice_worker::hls::{HlsMediaRunner, MergedHlsStream};
use crate::voice_worker::ledger::DELIVERY_LEDGER_FILE;
use crate::voice_worker::openai::OpenAiVoiceClient;
use crate::voice_worker::webpush::{WebPushPayload, WebPushSender, WEB_PUSH_TTL_SECONDS};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
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
    prepared: Option<GeneratedVoiceTask>,
    playing_until_ts: f64,
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

#[derive(Debug, Clone, PartialEq)]
struct GeneratedVoiceTask {
    task: QueuedVoiceTask,
    audio_bytes: Vec<u8>,
    listener_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerStepReport {
    pub action: String,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FinalResponseProcessResult {
    stop_tts: bool,
    action: String,
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
                if let Some(prepared) = state.prepared.take() {
                    dropped.push(prepared.task);
                }
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
            reset_hls_playlist(&self.app_dir)?;
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

    pub fn enqueue_pending_ledger_for_delivery(&self) -> Result<usize, String> {
        let ledger = read_ledger_object(&self.app_dir.join(DELIVERY_LEDGER_FILE));
        let mut candidates = ledger
            .iter()
            .filter(|(_, row)| should_deliver_row(row))
            .map(|(message_id, row)| (message_id.clone(), ledger_sort_ts(row)))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice runtime lock poisoned")?;
        let existing = state
            .queue
            .iter()
            .map(|task| task.message_id.clone())
            .chain(state.generating.iter().map(|task| task.message_id.clone()))
            .chain(
                state
                    .prepared
                    .iter()
                    .map(|task| task.task.message_id.clone()),
            )
            .chain(state.playing.iter().map(|task| task.message_id.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        let mut added = 0usize;
        for (message_id, _) in candidates {
            if existing.contains(message_id.as_str()) {
                continue;
            }
            state.queue.push_back(QueuedVoiceTask {
                source_message_ids: vec![message_id.clone()],
                message_id,
            });
            added += 1;
        }
        Ok(added)
    }

    pub fn enqueue_test_announcement(&self, voice: String) -> Result<(String, usize), String> {
        let now = now_seconds();
        let message_id = format!("test-{}", (now * 1000.0).round() as i64);
        let task = QueuedVoiceTask {
            message_id: message_id.clone(),
            source_message_ids: vec![message_id.clone()],
        };
        let queue_depth = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "voice runtime lock poisoned")?;
            prune_listeners(&mut state.listeners, now);
            if state.listeners.is_empty() {
                return Err("no active listener".to_string());
            }
            state.queue.push_back(task);
            state.queue.len()
        };
        self.upsert_ledger_row(json!({
            "message_id": message_id,
            "session_id": "test-session",
            "session_display_name": "Codoxear",
            "message_class": "narration",
            "preview_text": "This is a Codoxear announcement test.",
            "notification_text": "Codoxear announcement test",
            "summary_text": "",
            "summary_status": "skipped",
            "narrated_status": "pending",
            "push_status": "skipped",
            "voice": voice,
            "created_ts": now,
            "updated_ts": now,
            "last_error": "",
        }))?;
        Ok((message_id, queue_depth))
    }

    pub fn worker_step<C, H, W>(
        &self,
        client: &C,
        hls_runner: &H,
        push_sender: &W,
        now_ts: f64,
    ) -> Result<Option<WorkerStepReport>, String>
    where
        C: OpenAiVoiceClient,
        H: HlsMediaRunner,
        W: WebPushSender,
    {
        self.enqueue_pending_ledger_for_delivery()?;
        if let Some(prepared) = self.take_appendable_prepared(now_ts)? {
            return self.append_prepared(prepared, hls_runner, now_ts).map(Some);
        }
        let task = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "voice runtime lock poisoned")?;
            prune_listeners(&mut state.listeners, now_ts);
            if state.listeners.is_empty() || state.generating.is_some() || state.prepared.is_some()
            {
                return Ok(None);
            }
            if state.playing_until_ts > now_ts {
                return Ok(None);
            }
            state.playing = None;
            let Some(task) = state.queue.pop_front() else {
                return Ok(None);
            };
            state.generating = Some(task.clone());
            task
        };
        let result = self.process_task(task.clone(), client, push_sender, now_ts);
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice runtime lock poisoned")?;
        if state
            .generating
            .as_ref()
            .is_some_and(|current| current.message_id == task.message_id)
        {
            state.generating = None;
        }
        drop(state);
        result.map(Some)
    }

    fn take_appendable_prepared(&self, now_ts: f64) -> Result<Option<GeneratedVoiceTask>, String> {
        let stale = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "voice runtime lock poisoned")?;
            prune_listeners(&mut state.listeners, now_ts);
            if state.playing_until_ts <= now_ts {
                state.playing = None;
            }
            let Some(prepared) = state.prepared.as_ref() else {
                return Ok(None);
            };
            if state.listeners.is_empty() || prepared.listener_epoch != state.listener_epoch {
                state.prepared.take()
            } else if state.playing.is_none() {
                return Ok(state.prepared.take());
            } else {
                return Ok(None);
            }
        };
        if let Some(prepared) = stale {
            self.mark_tasks_skipped_no_listener(&[prepared.task])?;
        }
        Ok(None)
    }

    fn process_task<C, W>(
        &self,
        task: QueuedVoiceTask,
        client: &C,
        push_sender: &W,
        now_ts: f64,
    ) -> Result<WorkerStepReport, String>
    where
        C: OpenAiVoiceClient,
        W: WebPushSender,
    {
        let Some(mut row) = self.ledger_row(&task.message_id) else {
            return Ok(WorkerStepReport {
                action: "missing".to_string(),
                message_id: task.message_id,
            });
        };
        let settings = load_worker_settings(&self.app_dir);
        let message_class = row
            .get("message_class")
            .and_then(Value::as_str)
            .unwrap_or("narration")
            .to_string();
        let source_text = compact_text(
            row.get("preview_text")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let session_id = row
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let session_display_name = row
            .get("session_display_name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Session")
            .to_string();
        let voice = row
            .get("voice")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| voice_for_session(&session_id, &session_display_name));
        self.patch_task_rows(&task, json!({"voice": voice.clone(), "last_error": ""}))?;

        if message_class == "final_response" {
            let final_result = self.process_final_push_and_summary(
                &task,
                &mut row,
                &settings,
                push_sender,
                now_ts,
                client,
            )?;
            if final_result.stop_tts {
                return Ok(WorkerStepReport {
                    action: final_result.action,
                    message_id: task.message_id,
                });
            }
            if !settings
                .get("tts_enabled_for_final_response")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                self.patch_task_rows(&task, json!({"narrated_status": "skipped"}))?;
                return Ok(WorkerStepReport {
                    action: "final-push".to_string(),
                    message_id: task.message_id,
                });
            }
        }

        if settings
            .get("tts_api_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            self.set_task_error(&task, "tts_api_key is required")?;
            return Ok(WorkerStepReport {
                action: "error".to_string(),
                message_id: task.message_id,
            });
        }
        if self.listener_epoch_if_active(now_ts)?.is_none() {
            self.mark_tasks_skipped_no_listener(std::slice::from_ref(&task))?;
            return Ok(WorkerStepReport {
                action: "skipped".to_string(),
                message_id: task.message_id,
            });
        }
        let spoken_text = if message_class == "narration" {
            let summary = client.summarize(
                summary_request(&settings, &session_display_name, "Narration updates", 15),
                &source_text,
            );
            match summary {
                Ok(summary) => {
                    self.patch_task_rows(
                        &task,
                        json!({"summary_status": "sent", "summary_text": summary}),
                    )?;
                    format!("From {session_display_name}. {summary}")
                }
                Err(error) => {
                    self.set_task_error(&task, &error)?;
                    return Ok(WorkerStepReport {
                        action: "error".to_string(),
                        message_id: task.message_id,
                    });
                }
            }
        } else {
            let summary_text = self
                .ledger_row(&task.message_id)
                .and_then(|row| {
                    row.get("summary_text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            let basis = if summary_text.trim().is_empty() {
                source_text
            } else {
                summary_text
            };
            format!("Turn summary from {session_display_name}. {basis}")
        };
        let audio = match client.synthesize(tts_request(&settings, &voice), &spoken_text) {
            Ok(audio) => audio,
            Err(error) => {
                self.set_task_error(&task, &error)?;
                return Ok(WorkerStepReport {
                    action: "error".to_string(),
                    message_id: task.message_id,
                });
            }
        };
        let Some(listener_epoch) = self.listener_epoch_if_active(now_ts)? else {
            self.mark_tasks_skipped_no_listener(std::slice::from_ref(&task))?;
            return Ok(WorkerStepReport {
                action: "skipped".to_string(),
                message_id: task.message_id,
            });
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice runtime lock poisoned")?;
        state.prepared = Some(GeneratedVoiceTask {
            task: task.clone(),
            audio_bytes: audio,
            listener_epoch,
        });
        Ok(WorkerStepReport {
            action: "prepared".to_string(),
            message_id: task.message_id,
        })
    }

    fn process_final_push_and_summary<C, W>(
        &self,
        task: &QueuedVoiceTask,
        row: &mut Value,
        settings: &Value,
        push_sender: &W,
        now_ts: f64,
        client: &C,
    ) -> Result<FinalResponseProcessResult, String>
    where
        C: OpenAiVoiceClient,
        W: WebPushSender,
    {
        let source_text = compact_text(
            row.get("preview_text")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let fallback_text = clip_text(&source_text, 120);
        if settings
            .get("tts_api_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            self.patch_task_rows(
                task,
                json!({"summary_status": "skipped", "notification_text": fallback_text}),
            )?;
        } else {
            let session_name = row
                .get("session_display_name")
                .and_then(Value::as_str)
                .unwrap_or("Session");
            match client.summarize(
                summary_request(settings, session_name, "Final assistant response", 30),
                &source_text,
            ) {
                Ok(summary) => {
                    let notification_text = clip_text(&summary, 120);
                    self.patch_task_rows(
                        task,
                        json!({
                            "summary_status": "sent",
                            "summary_text": summary,
                            "notification_text": notification_text,
                        }),
                    )?;
                }
                Err(error) => {
                    let tts_enabled = settings
                        .get("tts_enabled_for_final_response")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    self.patch_task_rows(
                        task,
                        json!({
                            "summary_status": "error",
                            "notification_text": fallback_text,
                            "narrated_status": if tts_enabled { "error" } else { "skipped" },
                            "last_error": clip_text(&error, 400),
                        }),
                    )?;
                    self.set_last_error(&error);
                    self.send_final_response_push(task, row, push_sender, now_ts)?;
                    return Ok(FinalResponseProcessResult {
                        stop_tts: true,
                        action: "summary-error".to_string(),
                    });
                }
            }
        }
        self.send_final_response_push(task, row, push_sender, now_ts)?;
        Ok(FinalResponseProcessResult {
            stop_tts: false,
            action: "final-push".to_string(),
        })
    }

    fn send_final_response_push<W>(
        &self,
        task: &QueuedVoiceTask,
        row: &Value,
        sender: &W,
        now_ts: f64,
    ) -> Result<(), String>
    where
        W: WebPushSender,
    {
        let targets = read_mobile_push_targets(&self.app_dir);
        if targets.is_empty() {
            self.patch_task_rows(task, json!({"push_status": "skipped"}))?;
            return Ok(());
        }
        let payload = WebPushPayload::final_response_default(
            row.get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            row.get("session_display_name")
                .and_then(Value::as_str)
                .unwrap_or("Session"),
            &task.message_id,
            (now_ts * 1000.0).round() as i64,
        )
        .to_json_value();
        let subject = crate::voice_worker::vapid::default_vapid_subject_from_env();
        let mut records = read_subscription_map(&self.app_dir);
        let mut sent = 0usize;
        let mut failed = 0usize;
        let mut dropped = Vec::new();
        for target in targets {
            let endpoint = target.endpoint.clone();
            match sender.send_json(&target, &payload, WEB_PUSH_TTL_SECONDS, &subject) {
                Ok(()) => {
                    if let Some(record) = records.get_mut(&target.id).and_then(Value::as_object_mut)
                    {
                        record.insert("last_success_ts".to_string(), json!(now_ts));
                        record.insert("last_error".to_string(), json!(""));
                        record.insert("updated_ts".to_string(), json!(now_ts));
                    }
                    sent += 1;
                }
                Err(error) => {
                    if let Some(record) = records.get_mut(&target.id).and_then(Value::as_object_mut)
                    {
                        record.insert("last_failure_ts".to_string(), json!(now_ts));
                        record.insert(
                            "last_error".to_string(),
                            json!(clip_text(&format!("{error:?}"), 400)),
                        );
                        record.insert("updated_ts".to_string(), json!(now_ts));
                    }
                    if crate::voice_worker::webpush::should_drop_subscription(
                        &endpoint,
                        Some(&error),
                    ) {
                        dropped.push(target.id.clone());
                    }
                    failed += 1;
                }
            }
        }
        for id in dropped {
            records.remove(&id);
        }
        write_subscription_map(&self.app_dir, records)?;
        self.patch_task_rows(
            task,
            json!({"push_status": if sent > 0 { "sent" } else if failed > 0 { "error" } else { "skipped" }}),
        )?;
        Ok(())
    }

    fn append_prepared<H>(
        &self,
        prepared: GeneratedVoiceTask,
        runner: &H,
        now_ts: f64,
    ) -> Result<WorkerStepReport, String>
    where
        H: HlsMediaRunner,
    {
        let mut stream = MergedHlsStream::new(self.app_dir.join("audio"))?;
        match stream.append_audio(runner, &prepared.task.message_id, &prepared.audio_bytes) {
            Ok(duration) => {
                let snapshot = stream.snapshot();
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "voice runtime lock poisoned")?;
                state.segment_count = snapshot.segment_count;
                state.media_sequence = snapshot.media_sequence;
                state.last_error.clear();
                state.playing_until_ts = now_ts + duration.max(0.2);
                state.playing = Some(prepared.task.clone());
                drop(state);
                self.patch_task_rows(
                    &prepared.task,
                    json!({"narrated_status": "sent", "last_error": ""}),
                )?;
                Ok(WorkerStepReport {
                    action: "appended".to_string(),
                    message_id: prepared.task.message_id,
                })
            }
            Err(error) => {
                self.set_last_error(&error);
                self.set_task_error(&prepared.task, &error)?;
                Ok(WorkerStepReport {
                    action: "error".to_string(),
                    message_id: prepared.task.message_id,
                })
            }
        }
    }

    fn listener_epoch_if_active(&self, now_ts: f64) -> Result<Option<u64>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "voice runtime lock poisoned")?;
        prune_listeners(&mut state.listeners, now_ts);
        Ok((!state.listeners.is_empty()).then_some(state.listener_epoch))
    }

    fn ledger_row(&self, message_id: &str) -> Option<Value> {
        read_ledger_object(&self.app_dir.join(DELIVERY_LEDGER_FILE)).remove(message_id)
    }

    fn patch_task_rows(&self, task: &QueuedVoiceTask, patch: Value) -> Result<(), String> {
        self.patch_ledger_rows(&task.source_message_ids, patch)
    }

    fn patch_ledger_rows(&self, message_ids: &[String], patch: Value) -> Result<(), String> {
        let patch = patch.as_object().cloned().unwrap_or_default();
        let path = self.app_dir.join(DELIVERY_LEDGER_FILE);
        with_state_file_lock(&path, || {
            let mut ledger = read_ledger_object(&path);
            let now = now_seconds();
            let mut dirty = false;
            for id in message_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
            {
                let Some(row) = ledger.get_mut(id).and_then(Value::as_object_mut) else {
                    continue;
                };
                for (key, value) in &patch {
                    row.insert(key.clone(), value.clone());
                }
                row.insert("updated_ts".to_string(), json!(now));
                dirty = true;
            }
            if dirty {
                write_value(&path, &Value::Object(ledger))?;
            }
            Ok(())
        })
        .map_err(|(_, message)| message)
    }

    fn set_task_error(&self, task: &QueuedVoiceTask, error: &str) -> Result<(), String> {
        let clipped = clip_text(error, 400);
        self.set_last_error(&clipped);
        self.patch_task_rows(
            task,
            json!({
                "last_error": clipped,
                "narrated_status": "error",
                "summary_status": "error",
                "push_status": "error",
            }),
        )
    }

    fn upsert_ledger_row(&self, row: Value) -> Result<(), String> {
        let message_id = row
            .get("message_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "message_id required".to_string())?
            .to_string();
        let path = self.app_dir.join(DELIVERY_LEDGER_FILE);
        with_state_file_lock(&path, || {
            let mut ledger = read_ledger_object(&path);
            ledger.insert(message_id, row);
            write_value(&path, &Value::Object(ledger))
        })
        .map_err(|(_, message)| message)
    }

    pub fn set_last_error(&self, message: &str) {
        self.state.lock().unwrap().last_error = message.trim().to_string();
    }

    fn mark_tasks_skipped_no_listener(&self, tasks: &[QueuedVoiceTask]) -> Result<(), String> {
        let ids = collect_task_ids(tasks);
        if ids.is_empty() {
            return Ok(());
        }
        let path = delivery_ledger_path(&self.app_dir);
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
