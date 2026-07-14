use super::build::{apply_patch_file, build_text_diff, limit_model};
use super::model::{
    DiffLineKind, PatchFile, PatchFileKind, PermissionDiffFile, PermissionDiffModel,
    PermissionDiffWorkerInput, PermissionDiffWorkerOutput, PermissionEditOperation,
    PermissionFileChangeKind, SnapshotStatus,
};
use super::safety::{read_text_limited, resolve_path, ReadFailure};
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

const MAX_WORKER_STDIN_BYTES: u64 = 2 * 1024 * 1024;

pub fn run_stdio() -> Option<String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_WORKER_STDIN_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_WORKER_STDIN_BYTES {
        return None;
    }
    let input: PermissionDiffWorkerInput = serde_json::from_slice(&bytes).ok()?;
    let output = PermissionDiffWorkerOutput {
        request_id: input.request_id.clone(),
        model: enrich(input),
    };
    serde_json::to_string(&output).ok()
}

pub async fn spawn_worker(
    input: PermissionDiffWorkerInput,
) -> Result<PermissionDiffWorkerOutput, SnapshotStatus> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::process::Command;

    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(super::WORKER_TIMEOUT_MS);
    let binary = std::env::current_exe().map_err(|_| SnapshotStatus::Unreadable)?;
    let payload = serde_json::to_vec(&input).map_err(|_| SnapshotStatus::Unreadable)?;
    if payload.len() as u64 > MAX_WORKER_STDIN_BYTES {
        return Err(SnapshotStatus::TooLarge);
    }
    let mut child = Command::new(binary)
        .arg("__permission-diff-worker")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| SnapshotStatus::Unreadable)?;

    let mut stdin = child.stdin.take().ok_or(SnapshotStatus::Unreadable)?;
    let write_result = tokio::time::timeout_at(deadline, async {
        stdin.write_all(&payload).await?;
        stdin.shutdown().await
    })
    .await;
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(SnapshotStatus::Unreadable);
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(SnapshotStatus::Timeout);
        }
    }
    drop(stdin);

    let stdout = child.stdout.take().ok_or(SnapshotStatus::Unreadable)?;
    let output_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stdout
            .take(super::MAX_WORKER_STDOUT_BYTES + 1)
            .read_to_end(&mut output)
            .await
            .map(|_| output)
    });

    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => return Err(SnapshotStatus::Unreadable),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            output_task.abort();
            return Err(SnapshotStatus::Timeout);
        }
    };
    if !status.success() {
        output_task.abort();
        return Err(SnapshotStatus::Unreadable);
    }
    let output = output_task
        .await
        .map_err(|_| SnapshotStatus::Unreadable)?
        .map_err(|_| SnapshotStatus::Unreadable)?;
    if output.len() as u64 > super::MAX_WORKER_STDOUT_BYTES {
        return Err(SnapshotStatus::TooLarge);
    }
    serde_json::from_slice(&output).map_err(|_| SnapshotStatus::Unreadable)
}

fn enrich(input: PermissionDiffWorkerInput) -> PermissionDiffModel {
    let protected: HashSet<PathBuf> = input
        .protected_paths
        .iter()
        .filter_map(|path| resolve_path(&input.intent.workspace, path))
        .collect();
    let mut total_read = 0u64;
    let mut model = match &input.intent.operation {
        PermissionEditOperation::TextReplace {
            path,
            old_text,
            new_text,
            replace_all,
        } => enrich_text_replace(
            &input,
            path,
            old_text,
            new_text,
            *replace_all,
            &protected,
            &mut total_read,
        ),
        PermissionEditOperation::WholeFileWrite { path, content } => {
            enrich_write(&input, path, content, &protected, &mut total_read)
        }
        PermissionEditOperation::PatchSet { files } => {
            enrich_patch_set(&input, files, &protected, &mut total_read)
        }
        PermissionEditOperation::Unsupported { .. } => {
            PermissionDiffModel::unsupported(SnapshotStatus::Unsupported)
        }
    };
    model.request_id = input.request_id;
    model.snapshot_at_ms = Some(crate::history::now_ms().max(0) as u64);
    limit_model(model)
}

fn enrich_text_replace(
    input: &PermissionDiffWorkerInput,
    path: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
    protected: &HashSet<PathBuf>,
    total_read: &mut u64,
) -> PermissionDiffModel {
    if is_protected(input, path, protected) {
        return fallback(input, SnapshotStatus::ProtectedPath);
    }
    let source = match read_text_limited(&input.intent.workspace, path, protected, total_read) {
        Ok(source) => source,
        Err(error) => return fallback(input, error.status()),
    };
    if old_text.is_empty() {
        return fallback(input, SnapshotStatus::SourceMismatch);
    }
    let matches = source.match_indices(old_text).count();
    let after = if replace_all && matches > 0 {
        source.replace(old_text, new_text)
    } else if !replace_all && matches == 1 {
        source.replacen(old_text, new_text, 1)
    } else {
        return fallback(input, SnapshotStatus::SourceMismatch);
    };
    build_text_diff(
        Some(path),
        path,
        &source,
        &after,
        PermissionFileChangeKind::Modified,
        SnapshotStatus::SnapshotReady,
        true,
    )
}

fn enrich_write(
    input: &PermissionDiffWorkerInput,
    path: &str,
    content: &str,
    protected: &HashSet<PathBuf>,
    total_read: &mut u64,
) -> PermissionDiffModel {
    if is_protected(input, path, protected) {
        return fallback(input, SnapshotStatus::ProtectedPath);
    }
    match read_text_limited(&input.intent.workspace, path, protected, total_read) {
        Ok(source) => build_text_diff(
            Some(path),
            path,
            &source,
            content,
            PermissionFileChangeKind::Modified,
            SnapshotStatus::SnapshotReady,
            true,
        ),
        Err(ReadFailure::Missing) => build_text_diff(
            None,
            path,
            "",
            content,
            PermissionFileChangeKind::Added,
            SnapshotStatus::NewFile,
            true,
        ),
        Err(error) => fallback(input, error.status()),
    }
}

fn enrich_patch_set(
    input: &PermissionDiffWorkerInput,
    files: &[PatchFile],
    protected: &HashSet<PathBuf>,
    total_read: &mut u64,
) -> PermissionDiffModel {
    let initial_files = input
        .intent
        .initial_diff
        .as_ref()
        .map(|model| model.files.clone())
        .unwrap_or_default();
    let mut rendered = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let fallback_file = initial_files
            .get(index)
            .cloned()
            .unwrap_or_else(|| empty_file(file, SnapshotStatus::PayloadOnly));
        let relevant = file
            .old_path
            .as_deref()
            .into_iter()
            .chain(std::iter::once(file.new_path.as_str()));
        if relevant
            .into_iter()
            .any(|path| is_protected(input, path, protected))
        {
            rendered.push(with_status(fallback_file, SnapshotStatus::ProtectedPath));
            continue;
        }
        rendered.push(enrich_patch_file(
            input,
            file,
            fallback_file,
            protected,
            total_read,
        ));
    }
    let global = aggregate_status(&rendered);
    let mut model = PermissionDiffModel {
        request_id: input.request_id.clone(),
        snapshot_status: global,
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

fn enrich_patch_file(
    input: &PermissionDiffWorkerInput,
    file: &PatchFile,
    fallback_file: PermissionDiffFile,
    protected: &HashSet<PathBuf>,
    total_read: &mut u64,
) -> PermissionDiffFile {
    match file.kind {
        PatchFileKind::Add => {
            match read_text_limited(
                &input.intent.workspace,
                &file.new_path,
                protected,
                total_read,
            ) {
                Err(ReadFailure::Missing) => {
                    let content = added_content(file);
                    take_single_file(build_text_diff(
                        None,
                        &file.new_path,
                        "",
                        &content,
                        PermissionFileChangeKind::Added,
                        SnapshotStatus::NewFile,
                        true,
                    ))
                }
                Ok(_) => with_status(fallback_file, SnapshotStatus::SourceMismatch),
                Err(error) => with_status(fallback_file, error.status()),
            }
        }
        PatchFileKind::Update | PatchFileKind::Move => {
            let old_path = file.old_path.as_deref().unwrap_or(&file.new_path);
            let source =
                match read_text_limited(&input.intent.workspace, old_path, protected, total_read) {
                    Ok(source) => source,
                    Err(error) => return with_status(fallback_file, error.status()),
                };
            if file.kind == PatchFileKind::Move {
                match read_text_limited(
                    &input.intent.workspace,
                    &file.new_path,
                    protected,
                    total_read,
                ) {
                    Err(ReadFailure::Missing) => {}
                    _ => return with_status(fallback_file, SnapshotStatus::SourceMismatch),
                }
            }
            let Ok(after) = apply_patch_file(&source, file) else {
                return with_status(fallback_file, SnapshotStatus::SourceMismatch);
            };
            take_single_file(build_text_diff(
                (file.kind == PatchFileKind::Move).then_some(old_path),
                &file.new_path,
                &source,
                &after,
                if file.kind == PatchFileKind::Move {
                    PermissionFileChangeKind::Moved
                } else {
                    PermissionFileChangeKind::Modified
                },
                SnapshotStatus::SnapshotReady,
                true,
            ))
        }
        PatchFileKind::Delete => {
            let old_path = file.old_path.as_deref().unwrap_or(&file.new_path);
            match read_text_limited(&input.intent.workspace, old_path, protected, total_read) {
                Ok(source) => take_single_file(build_text_diff(
                    Some(old_path),
                    &file.new_path,
                    &source,
                    "",
                    PermissionFileChangeKind::Deleted,
                    SnapshotStatus::SnapshotReady,
                    true,
                )),
                Err(error) => with_status(fallback_file, error.status()),
            }
        }
    }
}

fn added_content(file: &PatchFile) -> String {
    let mut lines = Vec::new();
    for hunk in &file.hunks {
        for line in &hunk.lines {
            if line.kind == DiffLineKind::Add {
                lines.push(line.text.as_str());
            }
        }
    }
    let mut content = lines.join("\n");
    if !lines.is_empty() {
        content.push('\n');
    }
    content
}

fn fallback(input: &PermissionDiffWorkerInput, status: SnapshotStatus) -> PermissionDiffModel {
    fallback_model(&input.intent, &input.request_id, status)
}

pub fn fallback_model(
    intent: &super::PermissionEditIntent,
    request_id: &str,
    status: SnapshotStatus,
) -> PermissionDiffModel {
    let mut model = intent
        .initial_diff
        .clone()
        .unwrap_or_else(|| PermissionDiffModel::unsupported(status));
    model.request_id = request_id.to_string();
    model.snapshot_status = status;
    for file in &mut model.files {
        file.snapshot_status = status;
    }
    model
}

fn is_protected(
    input: &PermissionDiffWorkerInput,
    path: &str,
    protected: &HashSet<PathBuf>,
) -> bool {
    resolve_path(&input.intent.workspace, path)
        .is_some_and(|resolved| protected.contains(&resolved))
}

fn take_single_file(mut model: PermissionDiffModel) -> PermissionDiffFile {
    model
        .files
        .pop()
        .expect("text diff always contains one file")
}

fn with_status(mut file: PermissionDiffFile, status: SnapshotStatus) -> PermissionDiffFile {
    file.snapshot_status = status;
    file
}

fn empty_file(file: &PatchFile, status: SnapshotStatus) -> PermissionDiffFile {
    PermissionDiffFile {
        change_kind: match file.kind {
            PatchFileKind::Add => PermissionFileChangeKind::Added,
            PatchFileKind::Update => PermissionFileChangeKind::Modified,
            PatchFileKind::Delete => PermissionFileChangeKind::Deleted,
            PatchFileKind::Move => PermissionFileChangeKind::Moved,
        },
        old_path: file.old_path.clone(),
        new_path: file.new_path.clone(),
        snapshot_status: status,
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
        omitted_hunks: 0,
        omitted_lines: 0,
    }
}

fn aggregate_status(files: &[PermissionDiffFile]) -> SnapshotStatus {
    if files
        .iter()
        .any(|file| file.snapshot_status == SnapshotStatus::SnapshotReady)
    {
        SnapshotStatus::SnapshotReady
    } else if files
        .iter()
        .any(|file| file.snapshot_status == SnapshotStatus::NewFile)
    {
        SnapshotStatus::NewFile
    } else {
        files
            .first()
            .map(|file| file.snapshot_status)
            .unwrap_or(SnapshotStatus::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission_diff::adapters::{normalize_permission_edit, AdapterOutcome};
    use serde_json::json;
    use tempfile::tempdir;

    fn intent(
        agent: &str,
        tool: &str,
        value: serde_json::Value,
        workspace: &str,
    ) -> super::super::PermissionEditIntent {
        let AdapterOutcome::Intent(intent) =
            normalize_permission_edit(agent, tool, &value, workspace)
        else {
            panic!("expected intent");
        };
        *intent
    }

    #[test]
    fn enriches_claude_edit_from_snapshot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let input = PermissionDiffWorkerInput {
            request_id: "r1".into(),
            intent: intent(
                "claude",
                "Edit",
                json!({
                    "file_path": path,
                    "old_string": "two",
                    "new_string": "changed"
                }),
                dir.path().to_str().unwrap(),
            ),
            protected_paths: vec![],
        };
        let model = enrich(input);
        assert_eq!(model.snapshot_status, SnapshotStatus::SnapshotReady);
        assert_eq!(model.additions, 1);
        assert_eq!(model.deletions, 1);
        assert_eq!(model.request_id, "r1");
    }

    #[test]
    fn write_missing_file_becomes_new_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let input = PermissionDiffWorkerInput {
            request_id: "r2".into(),
            intent: intent(
                "claude",
                "Write",
                json!({"file_path": path, "content": "hello\n"}),
                dir.path().to_str().unwrap(),
            ),
            protected_paths: vec![],
        };
        let model = enrich(input);
        assert_eq!(model.snapshot_status, SnapshotStatus::NewFile);
        assert_eq!(model.additions, 1);
    }

    #[test]
    fn protected_path_never_reads_snapshot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "old").unwrap();
        let intent = intent(
            "claude",
            "Write",
            json!({"file_path": path, "content": "new"}),
            dir.path().to_str().unwrap(),
        );
        let resolved = resolve_path(&intent.workspace, path.to_str().unwrap()).unwrap();
        let model = enrich(PermissionDiffWorkerInput {
            request_id: "r3".into(),
            intent,
            protected_paths: vec![resolved.to_string_lossy().to_string()],
        });
        assert_eq!(model.snapshot_status, SnapshotStatus::ProtectedPath);
    }
}
