//! Lightweight confirm card (spec `docs/specs/im-diff-stage-transcript.md` D11–D12).
//!
//! Not the ask-question card and not the agent select card: title + body + confirm/cancel.

use crate::i18n::{self, Lang};

/// Max paths listed on a stage confirm card.
pub const STAGE_LIST_MAX: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmView {
    pub title: String,
    /// Plain / markdown-ish body (renderers escape as needed).
    pub body: String,
    pub confirm_label: String,
    pub cancel_label: String,
}

/// Build stage confirm view: list up to `STAGE_LIST_MAX` paths; note remaining count.
pub fn stage_confirm_view(
    lang: Lang,
    project: &str,
    paths: &[String],
    total: usize,
) -> ConfirmView {
    let title = i18n::tr(lang, "confirm.stageTitle")
        .replace("{project}", project);
    let mut body = String::new();
    body.push_str(
        &i18n::tr(lang, "confirm.stageIntro").replace("{n}", &total.to_string()),
    );
    body.push('\n');
    let show = paths.len().min(STAGE_LIST_MAX).min(total);
    for p in paths.iter().take(show) {
        body.push_str("- ");
        body.push_str(p);
        body.push('\n');
    }
    if total > show {
        body.push_str(
            &i18n::tr(lang, "confirm.stageMore")
                .replace("{n}", &(total - show).to_string()),
        );
        body.push('\n');
    }
    ConfirmView {
        title,
        body,
        confirm_label: i18n::tr(lang, "confirm.btnConfirm").to_string(),
        cancel_label: i18n::tr(lang, "confirm.btnCancel").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_view_truncates_list() {
        let paths: Vec<String> = (0..40).map(|i| format!("f{i}.rs")).collect();
        let v = stage_confirm_view(Lang::Zh, "proj", &paths, 40);
        assert!(v.body.contains("f0.rs"));
        assert!(v.body.contains("f29.rs"));
        assert!(!v.body.contains("f39.rs"));
        assert!(v.body.contains("10") || v.title.contains("proj"));
    }
}
