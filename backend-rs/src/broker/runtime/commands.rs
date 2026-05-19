use super::*;
use crate::broker::pi_live::coalesce_live_message_events;
use crate::broker::runtime_support::seq_bytes;
use crate::write_cleaners::normalize_pi_image_inputs;

pub(super) fn handle_send(req: &Value, state: &Arc<Mutex<State>>) -> Value {
    let Some(text) = req
        .get("text")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    else {
        return json!({"error": "text required"});
    };
    if state.lock().expect("broker state poisoned").backend == "pi" {
        let images = match normalize_pi_image_inputs(req.get("images").unwrap_or(&Value::Null)) {
            Ok(items) => items,
            Err(err) => return json!({"error": err}),
        };
        return handle_pi_send(text, images, state);
    }
    let mut st = state.lock().expect("broker state poisoned");
    st.busy = true;
    let enter = req
        .get("enter_seq")
        .and_then(Value::as_str)
        .map(seq_bytes)
        .unwrap_or_else(|| {
            seq_bytes(&std::env::var("CODEX_WEB_ENTER_SEQ").unwrap_or_else(|_| "\r".to_string()))
        });
    if let Some(master) = &mut st.pty_master {
        let _ = master.write_all(BRACKETED_PASTE_START);
        let _ = master.write_all(text.as_bytes());
        let _ = master.write_all(BRACKETED_PASTE_END);
        let _ = master.write_all(&enter);
    }
    json!({"queued": false, "queue_len": 0})
}

fn handle_pi_send(text: &str, images: Vec<Value>, state: &Arc<Mutex<State>>) -> Value {
    let (rpc, streaming_behavior) = {
        let mut st = state.lock().expect("broker state poisoned");
        let streaming_behavior = st.busy.then(|| "steer".to_string());
        st.busy = true;
        st.prompt_sent_at = Some(std::time::Instant::now());
        (st.pi_rpc.clone(), streaming_behavior)
    };
    let result = rpc
        .as_ref()
        .map(|rpc| rpc.prompt_with_options(text, streaming_behavior.as_deref(), Some(images)));
    {
        let mut st = state.lock().expect("broker state poisoned");
        if result.as_ref().is_some_and(Result::is_err) {
            st.busy = false;
            st.prompt_sent_at = None;
        }
        if let Some(Ok(value)) = &result {
            if let Some(turn_id) = value.get("turn_id").and_then(Value::as_str) {
                st.last_turn_id = Some(turn_id.to_string());
            }
            st.busy = true;
            st.prompt_sent_at = Some(std::time::Instant::now());
        }
    };
    match result {
        Some(Ok(_)) => json!({"queued": false, "queue_len": 0}),
        Some(Err(err)) => json!({"error": err}),
        None => json!({"error": "no state"}),
    }
}

pub(super) fn handle_keys(req: &Value, state: &Arc<Mutex<State>>) -> Value {
    let Some(seq) = req
        .get("seq")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    else {
        return json!({"error": "seq required"});
    };
    let bytes = seq_bytes(seq);
    let mut st = state.lock().expect("broker state poisoned");
    if st.backend == "pi" {
        if bytes != b"\x1b" {
            return json!({"error": format!("unsupported key sequence: {seq}")});
        }
        let (rpc, turn_id) = (st.pi_rpc.clone(), st.last_turn_id.clone());
        drop(st);
        let result = rpc.as_ref().map(|rpc| rpc.abort(turn_id.as_deref()));
        let mut st = state.lock().expect("broker state poisoned");
        match result {
            Some(Ok(_)) => {
                st.busy = false;
                st.last_turn_id = None;
                st.prompt_sent_at = None;
            }
            Some(Err(err)) => return json!({"error": err}),
            None => return json!({"error": "no state"}),
        }
    } else if let Some(master) = &mut st.pty_master {
        let _ = master.write_all(&bytes);
    }
    json!({"ok": true, "queued": false, "n": bytes.len()})
}

pub(super) fn handle_live_messages(req: &Value, state: &Arc<Mutex<State>>) -> Value {
    drain_pi_output(state);
    let since_offset = req.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let st = state.lock().expect("broker state poisoned");
    let rows = st
        .pi_live
        .message_events
        .iter()
        .filter(|(offset, _)| *offset > since_offset)
        .map(|(_, event)| event.clone())
        .collect::<Vec<_>>();
    json!({"offset": st.pi_live.message_offset, "events": coalesce_live_message_events(rows)})
}
