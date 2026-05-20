use crate::voice_worker::delivery::{
    clip_text, compact_text, read_mobile_push_targets, read_subscription_map, summary_request,
    write_subscription_map,
};
use crate::voice_worker::openai::OpenAiVoiceClient;
use crate::voice_worker::state::{FinalResponseProcessResult, QueuedVoiceTask, VoiceRuntime};
use crate::voice_worker::webpush::{WebPushPayload, WebPushSender, WEB_PUSH_TTL_SECONDS};
use serde_json::{json, Value};

impl VoiceRuntime {
    pub(crate) fn process_final_push_and_summary<C, W>(
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

    pub(crate) fn send_final_response_push<W>(
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
}
