use serde_json::{json, Value};

#[derive(Debug, Default)]
pub struct PiLiveState {
    pub message_offset: u64,
    pub message_events: Vec<(u64, Value)>,
    pub message_snapshots: serde_json::Map<String, Value>,
}

pub struct PiLiveRuntime<'a> {
    pub start_ts: f64,
    pub busy: &'a mut bool,
    pub last_turn_id: &'a mut Option<String>,
    pub pending_ui_requests: &'a mut serde_json::Map<String, Value>,
    pub live: &'a mut PiLiveState,
    pub output_tail: &'a mut String,
}

const TERMINAL_TURN_EVENT_TYPES: &[&str] = &[
    "thread_rolled_back",
    "turn_end",
    "turn.aborted",
    "turn.completed",
    "turn.failed",
];

const DIALOG_UI_METHODS: &[&str] = &["select", "confirm", "input", "editor"];

pub fn extract_turn_id(value: &Value) -> Option<&str> {
    for key in ["turn_id", "current_turn_id", "active_turn_id"] {
        if let Some(text) = value
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            return Some(text);
        }
    }
    value.get("payload").and_then(extract_turn_id)
}

pub fn extract_event_type(value: &Value) -> &str {
    value
        .get("type")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
}

pub fn event_output_text(event: &Value) -> Option<String> {
    let event_type = extract_event_type(event);
    if event_type == "message.delta" {
        return event
            .get("delta")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
    }
    if let Some(text) = event
        .get("text")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        let suffix = if text.ends_with('\n') { "" } else { "\n" };
        if event_type == "turn.started" {
            return Some(format!("> {text}{suffix}"));
        }
        return Some(format!("{text}{suffix}"));
    }
    if event_type == "tool.started" {
        return event
            .get("tool_name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(|name| format!("\n[tool] {name}\n"));
    }
    None
}

pub fn record_event(st: &mut PiLiveRuntime<'_>, event: &Value) {
    let event_type = extract_event_type(event);
    let event_turn_id = extract_turn_id(event).map(ToOwned::to_owned);
    let stream_id = live_stream_id(event_turn_id.as_deref());
    if event_type == "message.delta" {
        if let Some(delta) = event
            .get("delta")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            append_delta_snapshot(st, &stream_id, event_turn_id.as_deref(), event, delta);
        }
    }
    if event_type == "extension_ui_request" {
        record_ui_request(st, event);
    }
    for request_id in resolved_ui_request_ids(event) {
        (*st.pending_ui_requests).remove(&request_id);
    }
    let terminal_event = TERMINAL_TURN_EVENT_TYPES.contains(&event_type);
    let matches_active_turn = terminal_event
        && ((event_turn_id
            .as_deref()
            .map(|id| (*st.last_turn_id).as_deref() == Some(id))
            .unwrap_or(false))
            || (event_turn_id.is_none() && (*st.last_turn_id).is_none()));
    if matches_active_turn {
        (*st.pending_ui_requests).clear();
    }
    if terminal_event {
        if let Some(snapshot) = st.live.message_snapshots.get(&stream_id).cloned() {
            if snapshot
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|v| !v.is_empty())
            {
                let mut completed = snapshot;
                completed["completed"] = json!(true);
                append_live_message_event(st, completed);
            }
        }
        if matches_active_turn {
            (*st.busy) = false;
            (*st.last_turn_id) = None;
        }
    } else if let Some(turn_id) = event_turn_id {
        (*st.busy) = true;
        (*st.last_turn_id) = Some(turn_id);
    }
    if let Some(output) = event_output_text(event) {
        (*st.output_tail).push_str(&output);
    }
}

pub fn coalesce_live_message_events(rows: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut open_by_stream = std::collections::HashMap::<String, usize>::new();
    for row in rows {
        let stream_id = row
            .get("stream_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
        let completed = row.get("completed").and_then(Value::as_bool) == Some(true);
        if let Some(stream_id) = stream_id {
            if !completed {
                if let Some(index) = open_by_stream.get(&stream_id).copied() {
                    out[index] = row;
                } else {
                    open_by_stream.insert(stream_id, out.len());
                    out.push(row);
                }
            } else {
                open_by_stream.remove(&stream_id);
                out.push(row);
            }
        } else {
            out.push(row);
        }
    }
    out
}

fn live_stream_id(turn_id: Option<&str>) -> String {
    format!("pi-stream:{}", turn_id.unwrap_or("active"))
}

fn live_event_ts(st: &PiLiveRuntime<'_>, event: &Value) -> f64 {
    event
        .get("ts")
        .and_then(Value::as_f64)
        .unwrap_or(st.start_ts)
}

fn append_delta_snapshot(
    st: &mut PiLiveRuntime<'_>,
    stream_id: &str,
    turn_id: Option<&str>,
    event: &Value,
    delta: &str,
) {
    if !st.live.message_snapshots.contains_key(stream_id) {
        st.live.message_snapshots.insert(
            stream_id.to_string(),
            json!({"role":"assistant","text":"","streaming":true,"stream_id":stream_id,"turn_id":turn_id,"ts":live_event_ts(st, event)}),
        );
    }
    let mut snapshot = st
        .live
        .message_snapshots
        .get(stream_id)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let current = snapshot.get("text").and_then(Value::as_str).unwrap_or("");
    snapshot["text"] = json!(format!("{current}{delta}"));
    snapshot["turn_id"] = turn_id.map(|id| json!(id)).unwrap_or(Value::Null);
    st.live
        .message_snapshots
        .insert(stream_id.to_string(), snapshot.clone());
    append_live_message_event(st, snapshot);
}

fn append_live_message_event(st: &mut PiLiveRuntime<'_>, event: Value) {
    st.live.message_offset += 1;
    st.live.message_events.push((st.live.message_offset, event));
    if st.live.message_events.len() > 500 {
        let drop_count = st.live.message_events.len() - 500;
        st.live.message_events.drain(0..drop_count);
    }
}

fn record_ui_request(st: &mut PiLiveRuntime<'_>, event: &Value) {
    let Some(id) = event
        .get("id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    else {
        return;
    };
    let Some(method) = event.get("method").and_then(Value::as_str) else {
        return;
    };
    if !DIALOG_UI_METHODS.contains(&method) {
        return;
    }
    let mut req = json!({
        "id": id,
        "method": method,
        "title": event.get("title").cloned().unwrap_or(Value::Null),
        "message": event.get("message").cloned().unwrap_or(Value::Null),
        "question": event.get("question").and_then(Value::as_str),
        "context": event.get("context").and_then(Value::as_str),
        "options": event.get("options").and_then(Value::as_array).cloned().unwrap_or_default(),
        "allow_freeform": matches!(method, "select" | "input" | "editor"),
        "allow_multiple": false,
        "timeout_ms": event.get("timeout_ms").or_else(|| event.get("timeoutMs")).or_else(|| event.get("timeout")).cloned().unwrap_or(Value::Null),
        "status": "pending"
    });
    if let Some(value) = event
        .get("allow_freeform")
        .or_else(|| event.get("allowFreeform"))
    {
        req["allow_freeform"] = json!(value.as_bool().unwrap_or(false));
    }
    if let Some(value) = event
        .get("allow_multiple")
        .or_else(|| event.get("allowMultiple"))
    {
        req["allow_multiple"] = json!(value.as_bool().unwrap_or(false));
    }
    if let Some(prefill) = event.get("prefill").and_then(Value::as_str) {
        req["prefill"] = json!(prefill);
    }
    (*st.pending_ui_requests).insert(id.to_string(), req);
}

fn resolved_ui_request_ids(event: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_resolved(event, &mut out);
    if let Some(payload) = event.get("payload") {
        collect_resolved(payload, &mut out);
    }
    out
}

fn collect_resolved(value: &Value, out: &mut Vec<String>) {
    if let Some(id) = ask_user_request_id(value) {
        out.push(id);
    }
    if let Some(id) = value.get("message").and_then(ask_user_request_id) {
        out.push(id);
    }
    if let Some(items) = value.get("toolResults").and_then(Value::as_array) {
        for item in items {
            if let Some(id) = ask_user_request_id(item) {
                out.push(id);
            }
        }
    }
}

fn ask_user_request_id(value: &Value) -> Option<String> {
    (value.get("role").and_then(Value::as_str) == Some("toolResult")
        && value.get("toolName").and_then(Value::as_str) == Some("ask_user"))
    .then(|| value.get("toolCallId").and_then(Value::as_str))
    .flatten()
    .filter(|v| !v.is_empty())
    .map(ToOwned::to_owned)
}
