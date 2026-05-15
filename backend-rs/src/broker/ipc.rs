use serde_json::{json, Value};

#[derive(Debug, Clone, Default)]
pub struct BrokerSnapshot {
    pub busy: bool,
    pub queue_len: usize,
    pub token: Option<Value>,
    pub tail: String,
}

pub fn handle_snapshot_command(req: &Value, snapshot: &BrokerSnapshot) -> Value {
    match req.get("cmd").and_then(Value::as_str) {
        Some("state") => json!({
            "busy": snapshot.busy,
            "queue_len": snapshot.queue_len,
            "token": snapshot.token,
        }),
        Some("tail") => json!({"tail": snapshot.tail}),
        Some("send") => match req.get("text").and_then(Value::as_str) {
            Some(text) if !text.trim().is_empty() => json!({"queued": false, "queue_len": 0}),
            _ => json!({"error": "text required"}),
        },
        Some("keys") => match req.get("seq").and_then(Value::as_str) {
            Some(seq) if !seq.is_empty() => json!({"ok": true, "queued": false, "n": seq.len()}),
            _ => json!({"error": "seq required"}),
        },
        Some("shutdown") => json!({"ok": true}),
        _ => json!({"error": "unknown cmd"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_errors_match_python_broker_strings() {
        let snapshot = BrokerSnapshot::default();
        assert_eq!(
            handle_snapshot_command(&json!({"cmd": "send", "text": ""}), &snapshot),
            json!({"error": "text required"})
        );
        assert_eq!(
            handle_snapshot_command(&json!({"cmd": "keys"}), &snapshot),
            json!({"error": "seq required"})
        );
        assert_eq!(
            handle_snapshot_command(&json!({"cmd": "wat"}), &snapshot),
            json!({"error": "unknown cmd"})
        );
    }
}
