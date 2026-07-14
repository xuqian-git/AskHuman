use super::model::{
    DiffLineKind, PatchFile, PatchFileKind, PermissionDiffFile, PermissionDiffHunk,
    PermissionDiffLine, PermissionDiffModel, PermissionEditOperation, PermissionFileChangeKind,
    SnapshotStatus,
};
use similar::{Algorithm, ChangeTag, TextDiff};
use std::time::Duration;

const CONTEXT_LINES: usize = 3;
const PAYLOAD_DIFF_TIMEOUT_MS: u64 = 20;

pub fn initial_diff(operation: &PermissionEditOperation) -> Option<PermissionDiffModel> {
    let model = match operation {
        PermissionEditOperation::TextReplace {
            path,
            old_text,
            new_text,
            ..
        } => build_text_diff(
            None,
            path,
            old_text,
            new_text,
            PermissionFileChangeKind::Modified,
            SnapshotStatus::PayloadOnly,
            false,
        ),
        PermissionEditOperation::WholeFileWrite { path, content } => {
            proposed_content(path, content)
        }
        PermissionEditOperation::PatchSet { files } => patch_payload_diff(files),
        PermissionEditOperation::Unsupported { .. } => {
            PermissionDiffModel::unsupported(SnapshotStatus::Unsupported)
        }
    };
    Some(limit_model(model))
}

pub fn build_text_diff(
    old_path: Option<&str>,
    new_path: &str,
    old: &str,
    new: &str,
    change_kind: PermissionFileChangeKind,
    status: SnapshotStatus,
    include_line_numbers: bool,
) -> PermissionDiffModel {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(Duration::from_millis(PAYLOAD_DIFF_TIMEOUT_MS))
        .diff_lines(old, new);
    let mut hunks = Vec::new();
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for group in diff.grouped_ops(CONTEXT_LINES) {
        let mut lines = Vec::new();
        for op in group {
            for change in diff.iter_changes(&op) {
                let kind = match change.tag() {
                    ChangeTag::Equal => DiffLineKind::Context,
                    ChangeTag::Delete => {
                        deletions += 1;
                        DiffLineKind::Delete
                    }
                    ChangeTag::Insert => {
                        additions += 1;
                        DiffLineKind::Add
                    }
                };
                lines.push(PermissionDiffLine {
                    kind,
                    old_line: include_line_numbers
                        .then(|| change.old_index().map(|index| index + 1))
                        .flatten(),
                    new_line: include_line_numbers
                        .then(|| change.new_index().map(|index| index + 1))
                        .flatten(),
                    text: trim_line_ending(change.value()).to_string(),
                });
                if change.missing_newline() {
                    lines.push(PermissionDiffLine {
                        kind: DiffLineKind::Meta,
                        old_line: None,
                        new_line: None,
                        text: "No newline at end of file".to_string(),
                    });
                }
            }
        }
        if lines.iter().all(|line| line.kind == DiffLineKind::Context) {
            continue;
        }
        let old_start = include_line_numbers
            .then(|| lines.iter().find_map(|line| line.old_line))
            .flatten();
        let new_start = include_line_numbers
            .then(|| lines.iter().find_map(|line| line.new_line))
            .flatten();
        hunks.push(PermissionDiffHunk {
            old_start,
            new_start,
            header: String::new(),
            lines,
        });
    }

    let file = PermissionDiffFile {
        change_kind,
        old_path: old_path.map(str::to_string),
        new_path: new_path.to_string(),
        snapshot_status: status,
        hunks,
        additions,
        deletions,
        omitted_hunks: 0,
        omitted_lines: 0,
    };
    let mut model = PermissionDiffModel {
        request_id: String::new(),
        snapshot_status: status,
        snapshot_at_ms: None,
        files: vec![file],
        total_files: 1,
        additions,
        deletions,
        omitted_files: 0,
        omitted_hunks: 0,
        omitted_lines: 0,
        truncated: false,
    };
    model.recount();
    model
}

fn proposed_content(path: &str, content: &str) -> PermissionDiffModel {
    let lines = content
        .lines()
        .enumerate()
        .map(|(index, text)| PermissionDiffLine {
            kind: DiffLineKind::Context,
            old_line: None,
            new_line: Some(index + 1),
            text: text.to_string(),
        })
        .collect();
    let file = PermissionDiffFile {
        change_kind: PermissionFileChangeKind::Proposed,
        old_path: Some(path.to_string()),
        new_path: path.to_string(),
        snapshot_status: SnapshotStatus::PayloadOnly,
        hunks: vec![PermissionDiffHunk {
            old_start: None,
            new_start: Some(1),
            header: String::new(),
            lines,
        }],
        additions: 0,
        deletions: 0,
        omitted_hunks: 0,
        omitted_lines: 0,
    };
    let mut model = PermissionDiffModel {
        request_id: String::new(),
        snapshot_status: SnapshotStatus::PayloadOnly,
        snapshot_at_ms: None,
        files: vec![file],
        total_files: 1,
        additions: 0,
        deletions: 0,
        omitted_files: 0,
        omitted_hunks: 0,
        omitted_lines: 0,
        truncated: false,
    };
    model.recount();
    model
}

fn patch_payload_diff(files: &[PatchFile]) -> PermissionDiffModel {
    let mut rendered = Vec::with_capacity(files.len());
    for file in files {
        let mut additions = 0usize;
        let mut deletions = 0usize;
        let hunks = file
            .hunks
            .iter()
            .map(|hunk| {
                let lines = hunk
                    .lines
                    .iter()
                    .map(|line| {
                        match line.kind {
                            DiffLineKind::Add => additions += 1,
                            DiffLineKind::Delete => deletions += 1,
                            _ => {}
                        }
                        PermissionDiffLine {
                            kind: line.kind,
                            old_line: None,
                            new_line: None,
                            text: line.text.clone(),
                        }
                    })
                    .collect();
                PermissionDiffHunk {
                    old_start: None,
                    new_start: None,
                    header: hunk.header.clone(),
                    lines,
                }
            })
            .collect();
        rendered.push(PermissionDiffFile {
            change_kind: match file.kind {
                PatchFileKind::Add => PermissionFileChangeKind::Added,
                PatchFileKind::Update => PermissionFileChangeKind::Modified,
                PatchFileKind::Delete => PermissionFileChangeKind::Deleted,
                PatchFileKind::Move => PermissionFileChangeKind::Moved,
            },
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            snapshot_status: SnapshotStatus::PayloadOnly,
            hunks,
            additions,
            deletions,
            omitted_hunks: 0,
            omitted_lines: 0,
        });
    }
    let mut model = PermissionDiffModel {
        request_id: String::new(),
        snapshot_status: SnapshotStatus::PayloadOnly,
        snapshot_at_ms: None,
        total_files: rendered.len(),
        files: rendered,
        additions: 0,
        deletions: 0,
        omitted_files: 0,
        omitted_hunks: 0,
        omitted_lines: 0,
        truncated: false,
    };
    model.recount();
    model
}

pub fn apply_patch_file(source: &str, file: &PatchFile) -> Result<String, ()> {
    let mut current: Vec<String> = source.lines().map(str::to_string).collect();
    let mut cursor = 0usize;
    for hunk in &file.hunks {
        let old_block: Vec<&str> = hunk
            .lines
            .iter()
            .filter(|line| matches!(line.kind, DiffLineKind::Context | DiffLineKind::Delete))
            .map(|line| line.text.as_str())
            .collect();
        let new_block: Vec<String> = hunk
            .lines
            .iter()
            .filter(|line| matches!(line.kind, DiffLineKind::Context | DiffLineKind::Add))
            .map(|line| line.text.clone())
            .collect();
        if old_block.is_empty() {
            return Err(());
        }
        let start = find_block(&current, &old_block, cursor).ok_or(())?;
        let end = start + old_block.len();
        current.splice(start..end, new_block);
        cursor = start;
    }
    let mut result = current.join("\n");
    if source.ends_with('\n') || file.kind == PatchFileKind::Add {
        result.push('\n');
    }
    Ok(result)
}

fn find_block(haystack: &[String], needle: &[&str], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (from..=haystack.len().saturating_sub(needle.len())).find(|start| {
        haystack[*start..*start + needle.len()]
            .iter()
            .map(String::as_str)
            .eq(needle.iter().copied())
    })
}

pub fn limit_model(mut model: PermissionDiffModel) -> PermissionDiffModel {
    let mut global_remaining = super::MAX_DIFF_LINES_TOTAL;
    for file in &mut model.files {
        let mut file_remaining = super::MAX_DIFF_LINES_PER_FILE;
        let mut kept = Vec::new();
        for hunk in file.hunks.drain(..) {
            let line_count = hunk.lines.len();
            if line_count <= file_remaining && line_count <= global_remaining {
                file_remaining -= line_count;
                global_remaining -= line_count;
                kept.push(hunk);
            } else {
                file.omitted_hunks += 1;
                file.omitted_lines += line_count;
            }
        }
        file.hunks = kept;
    }
    model.recount();
    model
}

fn trim_line_ending(value: &str) -> &str {
    value
        .strip_suffix('\n')
        .unwrap_or(value)
        .strip_suffix('\r')
        .unwrap_or_else(|| value.strip_suffix('\n').unwrap_or(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission_diff::patch::parse_apply_patch;

    #[test]
    fn text_diff_has_add_delete_and_context() {
        let model = build_text_diff(
            None,
            "a.txt",
            "one\ntwo\nthree\n",
            "one\nchanged\nthree\n",
            PermissionFileChangeKind::Modified,
            SnapshotStatus::SnapshotReady,
            true,
        );
        assert_eq!(model.additions, 1);
        assert_eq!(model.deletions, 1);
        assert_eq!(model.files[0].hunks.len(), 1);
        assert!(model.files[0].hunks[0]
            .lines
            .iter()
            .any(|line| line.old_line == Some(2)));
    }

    #[test]
    fn applies_codex_update_patch_exactly() {
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n one\n-two\n+changed\n three\n*** End Patch\n";
        let files = parse_apply_patch(patch, 64).unwrap();
        let after = apply_patch_file("one\ntwo\nthree\n", &files[0]).unwrap();
        assert_eq!(after, "one\nchanged\nthree\n");
    }

    #[test]
    fn truncates_only_at_hunk_boundaries() {
        let mut model = build_text_diff(
            None,
            "a.txt",
            "a\n",
            &format!("{}\n", (0..500).map(|_| "b").collect::<Vec<_>>().join("\n")),
            PermissionFileChangeKind::Modified,
            SnapshotStatus::SnapshotReady,
            true,
        );
        model = limit_model(model);
        assert!(model.truncated);
        assert!(model.files[0].hunks.is_empty());
        assert!(model.omitted_lines > 400);
    }
}
