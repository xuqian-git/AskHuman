//! Diff → docx with red/green line backgrounds (Slack / DingTalk preview).

use crate::dingtalk::docx;
use crate::gitutil::{DiffModel, LineKind};

pub fn render(model: &DiffModel, meta_title: &str) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    body.push_str(&docx::ooxml_heading(1, meta_title));
    body.push_str(&docx::ooxml_para(&format!(
        "{} · {} file(s){}",
        model.git_root.display(),
        model.total_paths,
        if model.truncated { " · truncated" } else { "" }
    )));

    for f in &model.files {
        body.push_str(&docx::ooxml_heading(
            2,
            &format!("{} ({:?})", f.path, f.kind),
        ));
        if f.skipped && f.lines.is_empty() {
            body.push_str(&docx::ooxml_para(
                f.skip_reason.as_deref().unwrap_or("skipped"),
            ));
            continue;
        }
        for line in &f.lines {
            let (text, fill, color) = match line.kind {
                LineKind::Insert => {
                    let t = format!("+{}", line.text);
                    // GitHub-ish green bg / dark green text
                    (t, Some("E6FFEC"), Some("116329"))
                }
                LineKind::Delete => {
                    let t = format!("-{}", line.text);
                    (t, Some("FFEBE9"), Some("CF222E"))
                }
                LineKind::Equal => {
                    let t = format!(" {}", line.text);
                    (t, Some("F6F8FA"), Some("1F2328"))
                }
                LineKind::Header => {
                    // Only keep @@ hunk headers (same as md export).
                    if !line.text.starts_with("@@") {
                        continue;
                    }
                    (line.text.clone(), Some("F6F8FA"), Some("656D76"))
                }
            };
            body.push_str(&docx::ooxml_mono_line(&text, fill, color));
        }
        // spacer
        body.push_str(&docx::ooxml_para(""));
    }

    docx::build_raw_docx(&body).map_err(|e| e.to_string())
}
