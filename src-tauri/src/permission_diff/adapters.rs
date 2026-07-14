use super::model::{PermissionEditIntent, PermissionEditOperation, UnsupportedEditReason};
use serde_json::Value;

pub enum AdapterOutcome {
    Intent(Box<PermissionEditIntent>),
    NotNativeEdit,
}

pub fn normalize_permission_edit(
    agent: &str,
    tool_name: &str,
    tool_input: &Value,
    workspace: &str,
) -> AdapterOutcome {
    let operation = match (agent, tool_name) {
        ("claude", "Edit") => parse_claude_edit(tool_input),
        ("claude", "Write") => parse_claude_write(tool_input),
        ("claude", "NotebookEdit") => Some(PermissionEditOperation::Unsupported {
            reason: UnsupportedEditReason::NotebookEdit,
        }),
        ("codex", "apply_patch") => parse_codex_patch(tool_input),
        _ => return AdapterOutcome::NotNativeEdit,
    }
    .unwrap_or(PermissionEditOperation::Unsupported {
        reason: UnsupportedEditReason::InvalidPayload,
    });
    let initial_diff = super::build::initial_diff(&operation);
    AdapterOutcome::Intent(Box::new(PermissionEditIntent {
        agent_kind: agent.to_string(),
        native_tool: tool_name.to_string(),
        workspace: workspace.to_string(),
        operation,
        initial_diff,
    }))
}

fn parse_claude_edit(input: &Value) -> Option<PermissionEditOperation> {
    Some(PermissionEditOperation::TextReplace {
        path: valid_path(input.get("file_path")?.as_str()?)?,
        old_text: input.get("old_string")?.as_str()?.to_string(),
        new_text: input.get("new_string")?.as_str()?.to_string(),
        replace_all: input
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_claude_write(input: &Value) -> Option<PermissionEditOperation> {
    Some(PermissionEditOperation::WholeFileWrite {
        path: valid_path(input.get("file_path")?.as_str()?)?,
        content: input.get("content")?.as_str()?.to_string(),
    })
}

fn parse_codex_patch(input: &Value) -> Option<PermissionEditOperation> {
    let command = input.get("command")?.as_str()?;
    let files = super::patch::parse_apply_patch(command, super::MAX_FILES).ok()?;
    Some(PermissionEditOperation::PatchSet { files })
}

fn valid_path(path: &str) -> Option<String> {
    (!path.is_empty() && !path.contains('\0') && path.chars().count() <= 8192)
        .then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission_diff::model::{PermissionEditOperation, SnapshotStatus};
    use serde_json::json;

    #[test]
    fn normalizes_claude_edit() {
        let AdapterOutcome::Intent(intent) = normalize_permission_edit(
            "claude",
            "Edit",
            &json!({
                "file_path": "/tmp/a.txt",
                "old_string": "old",
                "new_string": "new",
                "replace_all": true
            }),
            "/tmp",
        ) else {
            panic!("expected edit intent");
        };
        assert!(matches!(
            intent.operation,
            PermissionEditOperation::TextReplace {
                replace_all: true,
                ..
            }
        ));
        assert_eq!(
            intent.initial_diff.unwrap().snapshot_status,
            SnapshotStatus::PayloadOnly
        );
    }

    #[test]
    fn notebook_is_explicitly_unsupported() {
        let AdapterOutcome::Intent(intent) = normalize_permission_edit(
            "claude",
            "NotebookEdit",
            &json!({"notebook_path": "/tmp/a.ipynb"}),
            "/tmp",
        ) else {
            panic!("expected intent");
        };
        assert!(matches!(
            intent.operation,
            PermissionEditOperation::Unsupported {
                reason: UnsupportedEditReason::NotebookEdit
            }
        ));
    }

    #[test]
    fn malformed_codex_patch_falls_back_without_partial_diff() {
        let AdapterOutcome::Intent(intent) = normalize_permission_edit(
            "codex",
            "apply_patch",
            &json!({"command": "*** Begin Patch\n*** Add File: a\n+x"}),
            "/tmp",
        ) else {
            panic!("expected intent");
        };
        assert!(matches!(
            intent.operation,
            PermissionEditOperation::Unsupported {
                reason: UnsupportedEditReason::InvalidPayload
            }
        ));
    }
}
