//! Stage confirm card for Slack (Block Kit).

use crate::confirm::ConfirmView;
use serde_json::{json, Value};

pub fn build_blocks(view: &ConfirmView) -> (Value, String) {
    let blocks = json!([
        {
            "type": "header",
            "text": { "type": "plain_text", "text": truncate(&view.title, 150), "emoji": true }
        },
        {
            "type": "section",
            "text": { "type": "mrkdwn", "text": truncate(&view.body, 2900) }
        },
        {
            "type": "actions",
            "elements": [
                {
                    "type": "button",
                    "text": { "type": "plain_text", "text": truncate(&view.confirm_label, 75) },
                    "style": "primary",
                    "action_id": "confirm_ok",
                    "value": "ok"
                },
                {
                    "type": "button",
                    "text": { "type": "plain_text", "text": truncate(&view.cancel_label, 75) },
                    "action_id": "confirm_cancel",
                    "value": "cancel"
                }
            ]
        }
    ]);
    (blocks, view.title.clone())
}

pub fn build_final_blocks(title: &str, text: &str) -> (Value, String) {
    let blocks = json!([
        {
            "type": "section",
            "text": { "type": "mrkdwn", "text": format!("*{}*\n{}", truncate(title, 100), truncate(text, 2800)) }
        }
    ]);
    (blocks, title.to_string())
}

/// Parse Slack interaction payload → (message_ts, ok).
pub fn parse_confirm_action(payload: &Value) -> Option<(String, bool)> {
    let actions = payload.get("actions")?.as_array()?;
    let act = actions.first()?;
    let id = act.get("action_id")?.as_str()?;
    let ok = match id {
        "confirm_ok" => true,
        "confirm_cancel" => false,
        _ => return None,
    };
    let ts = payload
        .get("message")
        .and_then(|m| m.get("ts"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if ts.is_empty() {
        return None;
    }
    Some((ts, ok))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}
