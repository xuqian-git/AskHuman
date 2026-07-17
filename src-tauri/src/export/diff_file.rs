//! Plain unified-style diff for Feishu `.diff` attachments.
//!
//! Tracked files retain the headers and lines produced by `git diff`. Untracked
//! files get synthetic headers so the result reads like a normal Git patch.
//! Safety-limit omissions are kept as explicit notes; the export prioritizes
//! reviewability and is not guaranteed to be accepted by `git apply`.

use crate::gitutil::{DiffModel, FileChangeKind, FileDiff, LineKind};

pub fn render(model: &DiffModel) -> String {
    let mut out = String::new();

    for file in &model.files {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }

        if file_has_git_header(file) {
            render_lines(&mut out, file);
        } else {
            render_synthetic_file(&mut out, file);
        }

        if file.skipped && !file.lines.is_empty() {
            out.push_str("# AskHuman: ");
            out.push_str(file.skip_reason.as_deref().unwrap_or("content omitted"));
            out.push('\n');
        }
    }

    out
}

fn file_has_git_header(file: &FileDiff) -> bool {
    file.lines
        .iter()
        .any(|line| line.kind == LineKind::Header && line.text.starts_with("diff --git "))
}

fn render_lines(out: &mut String, file: &FileDiff) {
    for line in &file.lines {
        match line.kind {
            LineKind::Insert => out.push('+'),
            LineKind::Delete => out.push('-'),
            LineKind::Equal => out.push(' '),
            LineKind::Header => {}
        }
        out.push_str(&line.text);
        out.push('\n');
    }
}

fn render_synthetic_file(out: &mut String, file: &FileDiff) {
    out.push_str("diff --git a/");
    out.push_str(&file.path);
    out.push_str(" b/");
    out.push_str(&file.path);
    out.push('\n');

    if file.kind == FileChangeKind::Binary {
        out.push_str("Binary files a/");
        out.push_str(&file.path);
        out.push_str(" and b/");
        out.push_str(&file.path);
        out.push_str(" differ\n");
        return;
    }

    if file.kind == FileChangeKind::Untracked {
        out.push_str("new file\n--- /dev/null\n+++ b/");
        out.push_str(&file.path);
        out.push('\n');
        if !file.lines.is_empty() {
            out.push_str("@@ -0,0 +1,");
            out.push_str(&file.lines.len().to_string());
            out.push_str(" @@\n");
            render_lines(out, file);
        }
    }

    if file.lines.is_empty() || file.kind != FileChangeKind::Untracked {
        out.push_str("# AskHuman: ");
        out.push_str(file.skip_reason.as_deref().unwrap_or("content omitted"));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitutil::{DiffLine, FileDiff};
    use std::path::PathBuf;

    fn model(files: Vec<FileDiff>) -> DiffModel {
        DiffModel {
            git_root: PathBuf::from("/repo"),
            total_paths: files.len(),
            files,
            truncated: false,
        }
    }

    #[test]
    fn tracked_diff_keeps_git_headers_and_line_prefixes() {
        let rendered = render(&model(vec![FileDiff {
            path: "src/main.rs".into(),
            kind: FileChangeKind::Modified,
            lines: vec![
                DiffLine {
                    kind: LineKind::Header,
                    text: "diff --git a/src/main.rs b/src/main.rs".into(),
                },
                DiffLine {
                    kind: LineKind::Header,
                    text: "@@ -1 +1 @@".into(),
                },
                DiffLine {
                    kind: LineKind::Delete,
                    text: "old".into(),
                },
                DiffLine {
                    kind: LineKind::Insert,
                    text: "new".into(),
                },
            ],
            skipped: false,
            skip_reason: None,
        }]));

        assert_eq!(
            rendered,
            "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n"
        );
    }

    #[test]
    fn untracked_file_gets_readable_patch_headers() {
        let rendered = render(&model(vec![FileDiff {
            path: "notes.txt".into(),
            kind: FileChangeKind::Untracked,
            lines: vec![
                DiffLine {
                    kind: LineKind::Insert,
                    text: "one".into(),
                },
                DiffLine {
                    kind: LineKind::Insert,
                    text: "two".into(),
                },
            ],
            skipped: false,
            skip_reason: None,
        }]));

        assert_eq!(
            rendered,
            "diff --git a/notes.txt b/notes.txt\nnew file\n--- /dev/null\n+++ b/notes.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n"
        );
    }

    #[test]
    fn omitted_binary_is_still_visible() {
        let rendered = render(&model(vec![FileDiff {
            path: "asset.bin".into(),
            kind: FileChangeKind::Binary,
            lines: Vec::new(),
            skipped: true,
            skip_reason: Some("binary".into()),
        }]));

        assert_eq!(
            rendered,
            "diff --git a/asset.bin b/asset.bin\nBinary files a/asset.bin and b/asset.bin differ\n"
        );
    }
}
