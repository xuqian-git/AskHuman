//! Chat-style HTML for full agent transcripts.

use crate::agents::transcript_full::{TranscriptDoc, TranscriptEvent};
use crate::watch;

pub fn render(doc: &TranscriptDoc, meta_title: &str) -> String {
    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", esc(meta_title)));
    let mut notes = Vec::new();
    if doc.truncated_head {
        notes.push("earlier events truncated");
    }
    if doc.partial {
        notes.push("some events could not be parsed");
    }
    if !notes.is_empty() {
        body.push_str(&format!(
            "<p class=\"meta\"><strong>Note:</strong> {}</p>\n",
            esc(&notes.join("; "))
        ));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    body.push_str("<div class=\"thread\">\n");
    for ev in &doc.events {
        body.push_str(&render_event(ev, now));
    }
    body.push_str("</div>\n");
    wrap_html(meta_title, &body)
}

fn render_event(ev: &TranscriptEvent, now: u64) -> String {
    match ev {
        TranscriptEvent::UserText {
            text,
            at,
            at_label,
        } => format!(
            "<hr class=\"sep\"/><div class=\"bubble user\"><div class=\"role\">User{}</div><div class=\"content\">{}</div></div>\n",
            fmt_at(*at, at_label.as_deref(), now),
            esc_preserve(text)
        ),
        TranscriptEvent::AssistantText {
            text,
            at,
            at_label,
        } => format!(
            "<hr class=\"sep\"/><div class=\"bubble assistant\"><div class=\"role\">Assistant{}</div><div class=\"content\">{}</div></div>\n",
            fmt_at(*at, at_label.as_deref(), now),
            esc_preserve(text)
        ),
        TranscriptEvent::Thinking {
            text,
            at,
            at_label,
        } => format!(
            "<div class=\"thinking-one\">thinking{}: {}…</div>\n",
            fmt_at(*at, at_label.as_deref(), now),
            esc(&text.chars().take(120).collect::<String>())
        ),
        TranscriptEvent::ToolCall {
            args_summary,
            is_error,
            ..
        } => {
            let mark = if *is_error { "✕" } else { "●" };
            let (label, obj) =
                crate::agents::transcript_full::split_tool_line(args_summary);
            let body = match obj {
                Some(o) => format!("<b>{}</b>: <i>{}</i>", esc(label), esc(o)),
                None => format!("<b>{}</b>", esc(label)),
            };
            format!(
                "<div class=\"tool-line{}\">{} {}</div>\n",
                if *is_error { " error" } else { "" },
                mark,
                body
            )
        }
        TranscriptEvent::Meta(t) => format!("<div class=\"meta-ev\">{}</div>\n", esc(t)),
    }
}

fn fmt_at(at: Option<u64>, label: Option<&str>, now: u64) -> String {
    if let Some(l) = label.filter(|s| !s.is_empty()) {
        return format!(" · {}", esc(l));
    }
    match at {
        Some(ts) => format!(" · {}", watch::fmt_local_time(ts, now)),
        None => String::new(),
    }
}

fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            _ => c.to_string(),
        })
        .collect()
}

fn esc_preserve(s: &str) -> String {
    esc(s).replace('\n', "<br/>\n")
}

fn wrap_html(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<meta name="color-scheme" content="light dark"/>
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  margin: 0 auto; max-width: 820px; padding: 1.25rem; line-height: 1.5;
  background: #0d1117; color: #e6edf3; }}
h1 {{ font-size: 1.3rem; color: #e6edf3; }}
.meta, .muted, .meta-ev, .role {{ color: #8b949e; }}
.meta {{ font-size: 0.9rem; }}
.thread {{ display: flex; flex-direction: column; gap: 0.5rem; }}
.sep {{ border: none; border-top: 1px solid #30363d; margin: 0.75rem 0 0.35rem; }}
.bubble {{ background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 10px;
  padding: 0.75rem 1rem; border-left: 4px solid #30363d; }}
.user {{ border-left-color: #58a6ff; }}
.assistant {{ border-left-color: #3fb950; }}
.role {{ font-size: 0.75rem; font-weight: 600; margin-bottom: 0.35rem; }}
.content {{ color: #e6edf3; }}
.tool-line {{ font-size: 0.9rem; color: #c9d1d9; padding: 0.15rem 0; }}
.tool-line.error {{ color: #f85149; }}
.thinking-one {{ font-size: 0.85rem; color: #8b949e; font-style: italic; }}
.meta-ev {{ font-size: 0.8rem; }}
@media (prefers-color-scheme: light) {{
  body {{ background: #f0f2f5; color: #1f2328; }}
  h1 {{ color: #1f2328; }}
  .meta, .muted, .meta-ev, .role, .thinking-one {{ color: #656d76; }}
  .sep {{ border-top-color: #d0d7de; }}
  .bubble {{ background: #fff; color: #1f2328; border-color: #d0d7de; }}
  .user {{ border-left-color: #0969da; }}
  .assistant {{ border-left-color: #1a7f37; }}
  .content {{ color: #1f2328; }}
  .tool-line {{ color: #424a53; }}
  .tool-line.error {{ color: #cf222e; }}
}}
</style>
</head>
<body>
{body}
</body>
</html>
"#,
        title = esc(title),
        body = body
    )
}
