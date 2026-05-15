use crate::log_normalizer::pi;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

const CONTEXT_WINDOW_BASELINE_TOKENS: i64 = pi::CONTEXT_WINDOW_BASELINE_TOKENS;

#[derive(Debug, Clone, Serialize)]
pub struct MessagesPage {
    pub events: Vec<Value>,
    pub has_older: bool,
    pub offset: usize,
    pub limit: usize,
    pub token: Option<Value>,
    pub idle: Option<bool>,
}

pub struct ChatExtraction {
    pub events: Vec<Value>,
    pub token: Option<Value>,
    pub turn_start: bool,
    pub turn_end: bool,
    pub turn_aborted: bool,
}

pub fn messages_from_codex_log(
    path: &Path,
    offset: usize,
    limit: usize,
    _init: bool,
    before: Option<usize>,
) -> Result<MessagesPage, String> {
    let objects = read_jsonl_objects(path, usize::MAX)?;
    let extraction = extract_chat_events(&objects)?;
    let end = before
        .unwrap_or(extraction.events.len())
        .min(extraction.events.len());
    let start = offset.min(end);
    let stop = start.saturating_add(limit).min(end);
    Ok(MessagesPage {
        events: extraction.events[start..stop].to_vec(),
        has_older: stop < end,
        offset: start,
        limit,
        token: extraction.token,
        idle: idle_from_objects(&objects)?,
    })
}

pub fn idle_from_log(path: &Path, max_scan_bytes: usize) -> Result<Option<bool>, String> {
    let objects = read_jsonl_objects(path, max_scan_bytes)?;
    idle_from_objects(&objects)
}

pub fn last_chat_role_ts_from_log(
    path: &Path,
    max_scan_bytes: usize,
) -> Result<Option<(String, f64)>, String> {
    let mut scan = 256 * 1024;
    while scan <= max_scan_bytes {
        let objects = read_jsonl_objects(path, scan)?;
        if let Some(value) = last_chat_role_ts_from_objects(&objects)? {
            return Ok(Some(value));
        }
        scan = scan.saturating_mul(2);
        if scan == 0 {
            break;
        }
    }
    Ok(None)
}

pub fn token_snapshot_from_log(
    path: &Path,
    max_scan_bytes: usize,
) -> Result<Option<Value>, String> {
    let objects = read_jsonl_objects(path, max_scan_bytes)?;
    Ok(extract_token_update(&objects))
}

pub fn run_settings_from_log(
    path: &Path,
    max_scan_bytes: usize,
) -> Result<crate::log_normalizer::RunSettings, String> {
    let objects = read_jsonl_objects(path, max_scan_bytes)?;
    let mut provider = None;
    let mut model = None;
    let mut reasoning_effort = None;
    for obj in objects.iter().rev() {
        if obj.get("type").and_then(Value::as_str) != Some("turn_context") {
            continue;
        }
        let Some(payload) = obj.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if model.is_none() {
            model = payload
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        if reasoning_effort.is_none() {
            reasoning_effort = payload
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        if provider.is_none() {
            provider = payload
                .get("model_provider")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        break;
    }
    Ok((provider, model, reasoning_effort))
}

pub fn todo_snapshot_payload_for_session(_path: &Path, _agent_backend: &str) -> Value {
    json!({"available": false, "error": false, "items": []})
}

pub fn read_chat_events_from_tail(
    path: &Path,
    min_events: usize,
    max_scan_bytes: usize,
) -> Result<Vec<Value>, String> {
    let objects = read_jsonl_objects(path, max_scan_bytes)?;
    let events = extract_chat_events(&objects)?.events;
    let _ = min_events;
    Ok(events)
}

pub fn extract_chat_events(objects: &[Value]) -> Result<ChatExtraction, String> {
    let mut events = Vec::new();
    let mut turn_start = false;
    let mut turn_end = false;
    let mut turn_aborted = false;

    for obj in objects {
        match obj.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(text) = pi::pi_user_text(obj) {
                    turn_start = true;
                    let mut event = json!({"role": "user", "text": text});
                    insert_ts(&mut event, obj);
                    events.push(event);
                    continue;
                }
                if let Some(text) = pi::pi_assistant_text(obj) {
                    let final_turn = pi::pi_final_turn(obj);
                    if final_turn {
                        turn_end = true;
                    }
                    let class = if final_turn {
                        "final_response"
                    } else {
                        "narration"
                    };
                    let mut event = json!({
                        "role": "assistant",
                        "text": text,
                        "message_class": class,
                        "message_id": text_message_id(class, &text, event_ts(obj)),
                    });
                    insert_ts(&mut event, obj);
                    events.push(event);
                }
            }
            Some("event_msg") => {
                let payload = obj
                    .get("payload")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "invalid event_msg payload".to_string())?;
                match payload.get("type").and_then(Value::as_str) {
                    Some("user_message") => {
                        if let Some(message) = payload.get("message").and_then(Value::as_str) {
                            turn_start = true;
                            let mut event = json!({"role": "user", "text": message});
                            insert_ts(&mut event, obj);
                            events.push(event);
                        }
                    }
                    Some("agent_message") => {
                        if let Some(message) = payload.get("message").and_then(Value::as_str) {
                            if !message.trim().is_empty() {
                                let class = if payload.get("phase").and_then(Value::as_str)
                                    == Some("final_answer")
                                {
                                    "final_response"
                                } else {
                                    "narration"
                                };
                                if class == "final_response" {
                                    turn_end = true;
                                }
                                let mut event = json!({
                                    "role": "assistant",
                                    "text": message,
                                    "message_class": class,
                                    "message_id": text_message_id(class, message, event_ts(obj)),
                                });
                                insert_ts(&mut event, obj);
                                events.push(event);
                            }
                        }
                    }
                    Some("turn_aborted") => turn_aborted = true,
                    Some("task_complete") | Some("turn_complete") => turn_end = true,
                    _ => {}
                }
            }
            Some("response_item") => {
                let payload = obj
                    .get("payload")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "invalid response_item payload".to_string())?;
                match payload.get("type").and_then(Value::as_str) {
                    Some("message")
                        if payload.get("role").and_then(Value::as_str) == Some("assistant") =>
                    {
                        let parts = payload
                            .get("content")
                            .and_then(Value::as_array)
                            .ok_or_else(|| "invalid assistant message content".to_string())?;
                        let text = parts
                            .iter()
                            .filter(|part| {
                                part.get("type").and_then(Value::as_str) == Some("output_text")
                            })
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect::<String>();
                        if !text.is_empty() {
                            let class = if payload.get("phase").and_then(Value::as_str)
                                == Some("final_answer")
                                || payload.get("end_turn").and_then(Value::as_bool) == Some(true)
                            {
                                "final_response"
                            } else {
                                "narration"
                            };
                            if class == "final_response" {
                                turn_end = true;
                            }
                            let mut event = json!({
                                "role": "assistant",
                                "text": text,
                                "message_class": class,
                                "message_id": text_message_id(class, &text, event_ts(obj)),
                            });
                            insert_ts(&mut event, obj);
                            events.push(event);
                        }
                    }
                    Some("function_call") => {
                        let name = payload
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        let mut event = json!({"type": "tool", "name": name});
                        insert_ts(&mut event, obj);
                        events.push(event);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(ChatExtraction {
        events,
        token: extract_token_update(objects),
        turn_start,
        turn_end,
        turn_aborted,
    })
}

fn idle_from_objects(objects: &[Value]) -> Result<Option<bool>, String> {
    if objects.is_empty() {
        return Ok(None);
    }
    let mut saw_terminal_signal = false;
    let mut idle = true;
    for obj in objects {
        match obj.get("type").and_then(Value::as_str) {
            Some("message") => {
                if pi::pi_user_text(obj).is_some() {
                    saw_terminal_signal = true;
                    idle = false;
                } else if pi::pi_assistant_text(obj).is_some() {
                    saw_terminal_signal = true;
                    idle = pi::pi_final_turn(obj);
                } else if pi_message_keeps_turn_busy(obj) {
                    saw_terminal_signal = true;
                    idle = false;
                }
            }
            Some("event_msg") => {
                let payload = obj
                    .get("payload")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "invalid event_msg payload".to_string())?;
                match payload.get("type").and_then(Value::as_str) {
                    Some("user_message") => {
                        saw_terminal_signal = true;
                        idle = false;
                    }
                    Some("agent_message")
                        if payload
                            .get("message")
                            .and_then(Value::as_str)
                            .is_some_and(|message| !message.trim().is_empty()) =>
                    {
                        saw_terminal_signal = true;
                        idle = false;
                    }
                    Some("agent_reasoning") => {
                        saw_terminal_signal = true;
                        idle = false;
                    }
                    Some("turn_aborted")
                    | Some("thread_rolled_back")
                    | Some("task_complete")
                    | Some("turn_complete") => {
                        saw_terminal_signal = true;
                        idle = true;
                    }
                    _ => {}
                }
            }
            Some("response_item") => {
                let payload = obj
                    .get("payload")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "invalid response_item payload".to_string())?;
                match payload.get("type").and_then(Value::as_str) {
                    Some("message") if has_assistant_output_text(obj)? => {
                        saw_terminal_signal = true;
                        idle = payload.get("end_turn").and_then(Value::as_bool) == Some(true);
                    }
                    Some("reasoning")
                    | Some("function_call")
                    | Some("function_call_output")
                    | Some("custom_tool_call")
                    | Some("custom_tool_call_output")
                    | Some("web_search_call")
                    | Some("local_shell_call") => {
                        saw_terminal_signal = true;
                        idle = false;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(Some(if saw_terminal_signal { idle } else { true }))
}

fn last_chat_role_ts_from_objects(objects: &[Value]) -> Result<Option<(String, f64)>, String> {
    let mut last_user: Option<(usize, Option<f64>)> = None;
    let mut last_assistant: Option<(usize, Option<f64>)> = None;
    for (idx, obj) in objects.iter().enumerate() {
        match obj.get("type").and_then(Value::as_str) {
            Some("message") => {
                if pi::pi_user_text(obj).is_some() {
                    last_user = Some((idx, event_ts(obj)));
                } else if pi::pi_assistant_text(obj).is_some() || pi_message_keeps_turn_busy(obj) {
                    last_assistant = Some((idx, event_ts(obj)));
                }
            }
            Some("event_msg") => {
                let payload = obj
                    .get("payload")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "invalid event_msg payload".to_string())?;
                match payload.get("type").and_then(Value::as_str) {
                    Some("user_message")
                        if payload.get("message").and_then(Value::as_str).is_some() =>
                    {
                        last_user = Some((idx, event_ts(obj)));
                    }
                    Some("agent_message")
                        if payload
                            .get("message")
                            .and_then(Value::as_str)
                            .is_some_and(|message| !message.trim().is_empty()) =>
                    {
                        last_assistant = Some((idx, event_ts(obj)));
                    }
                    _ => {}
                }
            }
            Some("response_item") if has_assistant_output_text(obj)? => {
                last_assistant = Some((idx, event_ts(obj)));
            }
            _ => {}
        }
    }
    let best = match (last_user, last_assistant) {
        (Some(user), Some(assistant)) if assistant.0 > user.0 => Some(("assistant", assistant)),
        (Some(user), _) => Some(("user", user)),
        (None, Some(assistant)) => Some(("assistant", assistant)),
        (None, None) => None,
    };
    let Some((role, (_idx, Some(ts)))) = best else {
        return Ok(None);
    };
    Ok(Some((role.to_string(), ts)))
}

fn extract_token_update(objects: &[Value]) -> Option<Value> {
    for obj in objects.iter().rev() {
        if let Some(token) = pi::pi_token_update(obj, None) {
            return Some(token);
        }
        if obj.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let payload = obj.get("payload")?.as_object()?;
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let info = payload.get("info")?.as_object()?;
        let ctx = info.get("model_context_window")?.as_i64()?;
        let last = info.get("last_token_usage")?.as_object()?;
        let total_tokens = last.get("total_tokens")?.as_i64()?;
        return Some(json!({
            "context_window": ctx,
            "tokens_in_context": total_tokens,
            "tokens_remaining": (ctx - total_tokens).max(0),
            "percent_remaining": context_percent_remaining(total_tokens, ctx),
            "baseline_tokens": CONTEXT_WINDOW_BASELINE_TOKENS,
            "as_of": obj.get("timestamp").and_then(Value::as_str),
        }));
    }
    None
}

fn has_assistant_output_text(obj: &Value) -> Result<bool, String> {
    if obj.get("type").and_then(Value::as_str) == Some("message") {
        return Ok(pi::pi_assistant_text(obj).is_some());
    }
    let payload = obj
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| "invalid response_item payload".to_string())?;
    if payload.get("type").and_then(Value::as_str) != Some("message")
        || payload.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Ok(false);
    }
    let parts = payload
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "invalid assistant message content".to_string())?;
    Ok(parts.iter().any(|part| {
        part.get("type").and_then(Value::as_str) == Some("output_text")
            && part
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
    }))
}

fn pi_message_keeps_turn_busy(obj: &Value) -> bool {
    pi::pi_message_role(obj).as_deref() == Some("toolResult")
        || pi::pi_assistant_thinking_count(obj) > 0
        || pi::pi_assistant_tool_use_count(obj) > 0
}

fn insert_ts(event: &mut Value, obj: &Value) {
    if let Some(ts) = event_ts(obj) {
        event["ts"] = json!(ts);
    }
}

fn event_ts(obj: &Value) -> Option<f64> {
    if let Some(ts) = obj.get("ts").and_then(Value::as_f64) {
        return Some(ts);
    }
    let timestamp = obj.get("timestamp")?;
    if let Some(ts) = timestamp.as_f64() {
        return Some(ts);
    }
    timestamp.as_str().and_then(parse_iso8601_to_epoch)
}

fn parse_iso8601_to_epoch(value: &str) -> Option<f64> {
    if !value.ends_with('Z') {
        return None;
    }
    let trimmed = value.trim_end_matches('Z');
    let (date, time) = trimmed.split_once('T')?;
    let mut date_parts = date.split('-').filter_map(|part| part.parse::<i32>().ok());
    let year = date_parts.next()?;
    let month = date_parts.next()?;
    let day = date_parts.next()?;
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i32>().ok()?;
    let minute = time_parts.next()?.parse::<i32>().ok()?;
    let second_raw = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let (second_part, fractional_part) = second_raw.split_once('.').unwrap_or((second_raw, ""));
    let second = second_part.parse::<i32>().ok()?;
    let fractional = if fractional_part.is_empty() {
        0.0
    } else {
        if !fractional_part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let digits = fractional_part.len().min(9);
        let numerator = fractional_part[..digits].parse::<u64>().ok()? as f64;
        numerator / 10_f64.powi(digits as i32)
    };
    Some(
        days_from_civil(year, month, day) as f64 * 86_400.0
            + (hour * 3600 + minute * 60 + second) as f64
            + fractional,
    )
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe - 719468) as i64
}

fn text_message_id(message_class: &str, text: &str, ts: Option<f64>) -> String {
    let ts_ms = ts.map(|value| (value * 1000.0).round() as i64);
    let payload = json!({"class": message_class, "text": text.split_whitespace().collect::<Vec<_>>().join(" "), "ts_ms": ts_ms});
    stable_hash(&serde_json::to_string(&payload).unwrap())
}

fn stable_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn context_percent_remaining(tokens_in_context: i64, context_window: i64) -> i64 {
    if context_window <= CONTEXT_WINDOW_BASELINE_TOKENS {
        return 0;
    }
    let effective = context_window - CONTEXT_WINDOW_BASELINE_TOKENS;
    let used = (tokens_in_context - CONTEXT_WINDOW_BASELINE_TOKENS).max(0);
    let remaining = (effective - used).max(0);
    ((remaining as f64 / effective as f64) * 100.0).round() as i64
}

fn read_jsonl_objects(path: &Path, max_scan_bytes: usize) -> Result<Vec<Value>, String> {
    let raw = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let start = raw.len().saturating_sub(max_scan_bytes);
    let slice = if start > 0 {
        let offset = raw[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|pos| start + pos + 1)
            .unwrap_or(start);
        &raw[offset..]
    } else {
        &raw
    };
    Ok(slice
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(|byte| byte.is_ascii_whitespace()))
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(Value::is_object)
        .collect())
}
