use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionEditIntent {
    pub agent_kind: String,
    pub native_tool: String,
    pub workspace: String,
    pub operation: PermissionEditOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_diff: Option<PermissionDiffModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PermissionEditOperation {
    TextReplace {
        path: String,
        old_text: String,
        new_text: String,
        replace_all: bool,
    },
    WholeFileWrite {
        path: String,
        content: String,
    },
    PatchSet {
        files: Vec<PatchFile>,
    },
    Unsupported {
        reason: UnsupportedEditReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedEditReason {
    NotebookEdit,
    InvalidPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchFile {
    pub kind: PatchFileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub new_path: String,
    #[serde(default)]
    pub hunks: Vec<PatchHunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchFileKind {
    Add,
    Update,
    Delete,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchHunk {
    #[serde(default)]
    pub header: String,
    pub lines: Vec<PatchLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiffModel {
    #[serde(default)]
    pub request_id: String,
    pub snapshot_status: SnapshotStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_at_ms: Option<u64>,
    #[serde(default)]
    pub files: Vec<PermissionDiffFile>,
    pub total_files: usize,
    pub additions: usize,
    pub deletions: usize,
    #[serde(default)]
    pub omitted_files: usize,
    #[serde(default)]
    pub omitted_hunks: usize,
    #[serde(default)]
    pub omitted_lines: usize,
    #[serde(default)]
    pub truncated: bool,
}

impl PermissionDiffModel {
    pub fn unsupported(status: SnapshotStatus) -> Self {
        Self {
            request_id: String::new(),
            snapshot_status: status,
            snapshot_at_ms: None,
            files: Vec::new(),
            total_files: 0,
            additions: 0,
            deletions: 0,
            omitted_files: 0,
            omitted_hunks: 0,
            omitted_lines: 0,
            truncated: false,
        }
    }

    pub fn recount(&mut self) {
        self.total_files = self.files.len().saturating_add(self.omitted_files);
        self.additions = self.files.iter().map(|file| file.additions).sum();
        self.deletions = self.files.iter().map(|file| file.deletions).sum();
        self.omitted_hunks = self.files.iter().map(|file| file.omitted_hunks).sum();
        self.omitted_lines = self.files.iter().map(|file| file.omitted_lines).sum();
        self.truncated = self.omitted_files > 0 || self.omitted_hunks > 0 || self.omitted_lines > 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiffFile {
    pub change_kind: PermissionFileChangeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub new_path: String,
    pub snapshot_status: SnapshotStatus,
    #[serde(default)]
    pub hunks: Vec<PermissionDiffHunk>,
    pub additions: usize,
    pub deletions: usize,
    #[serde(default)]
    pub omitted_hunks: usize,
    #[serde(default)]
    pub omitted_lines: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionFileChangeKind {
    Added,
    Modified,
    Deleted,
    Moved,
    Proposed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiffHunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_start: Option<usize>,
    #[serde(default)]
    pub header: String,
    pub lines: Vec<PermissionDiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiffLine {
    pub kind: DiffLineKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
    Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    PayloadOnly,
    SnapshotReady,
    NewFile,
    ProtectedPath,
    Timeout,
    TooLarge,
    TooManyFiles,
    NonUtf8,
    NotRegularFile,
    Unreadable,
    SourceMismatch,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiffWorkerInput {
    pub request_id: String,
    pub intent: PermissionEditIntent,
    #[serde(default)]
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiffWorkerOutput {
    pub request_id: String,
    pub model: PermissionDiffModel,
}
