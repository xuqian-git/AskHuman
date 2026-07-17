//! Shared presentation and validation helpers for the one-shot IM `/msg` compose card.

use crate::i18n::{self, Lang};
use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const INPUT_MAX_CHARS: usize = 3000;
pub const PREVIEW_MAX_CHARS: usize = 1600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MsgComposePayload {
    pub session_id: String,
    pub expires_at: u64,
    #[serde(default)]
    pub recovered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MsgComposeRecovery {
    pub channel: String,
    pub message_id: String,
    pub session_id: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPreview {
    pub text: String,
    pub omitted_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgComposeView {
    pub seq: u64,
    pub title: String,
    pub target_label: String,
    pub target: String,
    pub pending_label: String,
    pub pending_preview: Option<PendingPreview>,
    pub preview_omitted: Option<String>,
    pub input_label: String,
    pub input_placeholder: String,
    pub send_label: String,
    pub error: Option<String>,
}

impl MsgComposeView {
    pub fn plain_body(&self) -> String {
        let mut parts = vec![
            format!("{}: {}", self.target_label, self.target),
            self.pending_label.clone(),
        ];
        if let Some(preview) = &self.pending_preview {
            parts.push(preview.text.clone());
            if let Some(omitted) = &self.preview_omitted {
                parts.push(omitted.clone());
            }
        }
        if let Some(error) = &self.error {
            parts.push(error.clone());
        }
        parts.join("\n\n")
    }
}

pub fn build_view(
    rec: &Value,
    pending_count: usize,
    pending_text: &str,
    error: Option<String>,
    lang: Lang,
) -> MsgComposeView {
    let seq = rec.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let kind = rec.get("kind").and_then(Value::as_str).unwrap_or("");
    let kind_label = crate::agents::AgentKind::parse(kind)
        .map(|value| value.label())
        .unwrap_or(kind);
    let title = i18n::tr(lang, "msgCard.title")
        .replace("{id}", &seq.to_string())
        .replace("{agent}", kind_label);
    let pending_label = if pending_count == 0 {
        i18n::tr(lang, "msgCard.pendingNone").to_string()
    } else {
        i18n::tr(lang, "msgCard.pendingCount").replace("{n}", &pending_count.to_string())
    };
    let pending_preview =
        (!pending_text.is_empty()).then(|| preview_pending(pending_text, PREVIEW_MAX_CHARS));
    let preview_omitted = pending_preview.as_ref().and_then(|preview| {
        (preview.omitted_chars > 0).then(|| {
            i18n::tr(lang, "msgCard.previewOmitted")
                .replace("{n}", &preview.omitted_chars.to_string())
        })
    });
    MsgComposeView {
        seq,
        title,
        target_label: i18n::tr(lang, "msgCard.target").to_string(),
        target: crate::autochannel::kind_title_project(rec, lang),
        pending_label,
        pending_preview,
        preview_omitted,
        input_label: i18n::tr(lang, "msgCard.inputLabel").to_string(),
        input_placeholder: i18n::tr(lang, "msgCard.inputPlaceholder").to_string(),
        send_label: i18n::tr(lang, "msgCard.send").to_string(),
        error,
    }
}

pub fn preview_pending(text: &str, max_chars: usize) -> PendingPreview {
    let total = text.chars().count();
    if total <= max_chars {
        return PendingPreview {
            text: text.to_string(),
            omitted_chars: 0,
        };
    }
    let head_len = max_chars.div_ceil(2);
    let tail_len = max_chars / 2;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text.chars().skip(total.saturating_sub(tail_len)).collect();
    PendingPreview {
        text: format!("{head}\n…\n{tail}"),
        omitted_chars: total.saturating_sub(head_len + tail_len),
    }
}

pub fn validate_input(input: Option<&str>, lang: Lang) -> Result<String, String> {
    let Some(input) = input.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(i18n::tr(lang, "msgCard.inputRequired").to_string());
    };
    if input.chars().count() > INPUT_MAX_CHARS {
        return Err(
            i18n::tr(lang, "msgCard.inputTooLong").replace("{n}", &INPUT_MAX_CHARS.to_string())
        );
    }
    Ok(input.to_string())
}

/// Escape user-controlled pending text before placing it in DingTalk's markdown template.
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '`'
                | '*'
                | '_'
                | '{'
                | '}'
                | '['
                | ']'
                | '('
                | ')'
                | '#'
                | '+'
                | '-'
                | '.'
                | '!'
                | '|'
                | '>'
                | '<'
                | '&'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn encode_payload(payload: &MsgComposePayload) -> Option<String> {
    serde_json::to_string(payload).ok()
}

pub fn decode_payload(payload: Option<&str>) -> Option<MsgComposePayload> {
    serde_json::from_str(payload?).ok()
}

/// 读取恢复账本；缺失、损坏或字段为空的记录直接忽略。
pub fn load_recovery() -> Vec<MsgComposeRecovery> {
    let Ok(text) = std::fs::read_to_string(paths::msg_compose_file()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<MsgComposeRecovery>>(&text)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            !item.channel.is_empty() && !item.message_id.is_empty() && !item.session_id.is_empty()
        })
        .collect()
}

/// 原子写入恢复账本。账本刻意不包含用户输入正文。
pub fn save_recovery(items: &[MsgComposeRecovery]) {
    let Ok(json) = serde_json::to_string_pretty(items) else {
        return;
    };
    let path = paths::msg_compose_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    if std::fs::write(&tmp, json.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_preview_keeps_both_ends() {
        let preview = preview_pending("abcdefghij", 6);
        assert_eq!(preview.text, "abc\n…\nhij");
        assert_eq!(preview.omitted_chars, 4);
    }

    #[test]
    fn pending_preview_preserves_short_layout() {
        let preview = preview_pending("a\n b", 10);
        assert_eq!(preview.text, "a\n b");
        assert_eq!(preview.omitted_chars, 0);
    }

    #[test]
    fn input_validation_trims_and_limits_unicode() {
        assert_eq!(validate_input(Some(" hi "), Lang::En).unwrap(), "hi");
        assert!(validate_input(Some("  "), Lang::En).is_err());
        assert!(validate_input(Some(&"你".repeat(INPUT_MAX_CHARS + 1)), Lang::Zh).is_err());
    }

    #[test]
    fn recovery_round_trip_contains_no_message_content() {
        let item = MsgComposeRecovery {
            channel: "feishu".into(),
            message_id: "om_1".into(),
            session_id: "session-1".into(),
            expires_at: 42,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("content"));
        assert!(!json.contains("draft"));
        assert_eq!(
            serde_json::from_str::<MsgComposeRecovery>(&json).unwrap(),
            item
        );
    }

    #[test]
    fn dingtalk_markdown_escape_neutralizes_user_formatting() {
        assert_eq!(escape_markdown("# <x> *bold*"), "\\# \\<x\\> \\*bold\\*");
    }
}
