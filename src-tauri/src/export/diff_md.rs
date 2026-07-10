//! Pure Markdown diff (Feishu preview; fenced ```diff blocks).
//!
//! Content is emitted **verbatim** (user定案：代码准确比高亮一致更重要).
//! Feishu may colour some hunks as whole-line red/green and others as
//! syntax highlight — that is client-side and not sanitized away.
//! Only git metadata headers (`diff --git` / `index` / `---` / `+++`) are
//! dropped; `@@` hunk headers and all body lines keep original text.

use crate::gitutil::{DiffModel, LineKind};

pub fn render(model: &DiffModel, meta_title: &str) -> String {
    let mut md = String::new();
    md.push_str("# ");
    md.push_str(meta_title);
    md.push_str("\n\n");
    md.push_str(&format!(
        "`{}` · {} file(s){}\n\n---\n\n",
        model.git_root.display(),
        model.total_paths,
        if model.truncated { " · truncated" } else { "" }
    ));
    for f in &model.files {
        md.push_str("## `");
        md.push_str(&f.path);
        md.push_str(&format!("` ({:?})\n\n", f.kind));
        if f.skipped && f.lines.is_empty() {
            md.push_str(&format!(
                "_{}_\n\n",
                f.skip_reason.as_deref().unwrap_or("skipped")
            ));
            continue;
        }
        md.push_str("```diff\n");
        for line in &f.lines {
            match line.kind {
                LineKind::Insert => {
                    md.push('+');
                    md.push_str(&line.text);
                    md.push('\n');
                }
                LineKind::Delete => {
                    md.push('-');
                    md.push_str(&line.text);
                    md.push('\n');
                }
                LineKind::Equal => {
                    md.push(' ');
                    md.push_str(&line.text);
                    md.push('\n');
                }
                LineKind::Header => {
                    // Keep only @@ hunk headers; drop diff/index/---/+++ metadata.
                    if line.text.starts_with("@@") {
                        md.push_str(&line.text);
                        md.push('\n');
                    }
                }
            }
        }
        md.push_str("```\n\n");
    }
    md
}
