//! Stage confirm card for Telegram (HTML + inline keyboard).

use crate::confirm::ConfirmView;
use crate::telegram::markdown;
use serde_json::{json, Value};

pub fn build_html(view: &ConfirmView) -> String {
    format!(
        "<b>{}</b>\n\n{}",
        markdown::escape_html(&view.title),
        markdown::escape_html(&view.body)
    )
}

pub fn inline_keyboard(view: &ConfirmView) -> Value {
    // Must wrap as InlineKeyboardMarkup (same as select/watch), not a bare button matrix.
    json!({
        "inline_keyboard": [[
            { "text": view.confirm_label, "callback_data": "confirm:ok" },
            { "text": view.cancel_label, "callback_data": "confirm:cancel" }
        ]]
    })
}

/// Parse callback_data `confirm:ok|cancel` → ok bool.
pub fn parse_confirm_action(data: &str) -> Option<bool> {
    match data {
        "confirm:ok" => Some(true),
        "confirm:cancel" => Some(false),
        _ => None,
    }
}
