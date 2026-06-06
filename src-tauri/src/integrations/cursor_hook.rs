//! Cursor Hook 集成：安装/移除/状态 + 跨平台打开 hooks.json。
//! hooks.json 的增删为纯函数，便于单测（幂等、保留他人条目）。

use crate::paths;
use anyhow::{Context, Result};
use serde_json::{json, Value};

/// 识别本应用条目的标记（脚本文件名）。
pub const MARKER: &str = "askhuman-timeout.sh";

/// 旧版标记，用于识别 / 清理历史安装（向后兼容）。
pub const LEGACY_MARKER: &str = "humaninloop-timeout.sh";

/// 钩子脚本内容（安装时写入并 chmod 0755）。grep 正则与 Swift 版逐字一致。
pub const SCRIPT_CONTENT: &str = r##"#!/usr/bin/env bash
# askhuman-timeout.sh
# 由 AskHuman 自动安装 / 移除，请勿手动编辑。
#
# 作用：作为 Cursor 的 preToolUse 钩子，检测 Shell 工具调用是否会执行 AskHuman
# 命令；命中时将工具调用 timeout 提升至 24 小时（86400000ms），防止等待用户回应
# 时被强制取消。未命中或解析失败时输出空对象 `{}`，保持 fail-open。

set -u

# 从 stdin 读取 JSON 输入
input=$(cat)

# 解析 .tool_input.command，按 python3 -> jq -> 简单 grep 顺序回退
command=""
if command -v python3 >/dev/null 2>&1; then
  command=$(printf '%s' "$input" | python3 -c '
import json
import sys

try:
    data = json.load(sys.stdin)
    print(data.get("tool_input", {}).get("command", ""))
except Exception:
    print("")
')
elif command -v jq >/dev/null 2>&1; then
  command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)
else
  command="$input"
fi

# 匹配 AskHuman 调用：兼顾行内任意位置 / 链式命令 / 引号包裹 / 绝对路径前缀
if [ -n "$command" ] && printf '%s' "$command" \
     | grep -Eq "(^|[[:space:];&|()\`\"'\\]|/)AskHuman([[:space:]]|$|[\"'\\])"; then
  output='{"updated_input": {"timeout": 86400000}}'
else
  output='{}'
fi

printf '%s\n' "$output"

exit 0
"##;

/// 当前平台是否支持 Cursor Hook（仅 unix：macOS/Linux；Windows 不支持）。
pub fn supported() -> bool {
    cfg!(unix)
}

/// hooks.json 是否存在。
pub fn hooks_json_exists() -> bool {
    paths::cursor_hooks_json().exists()
}

/// 是否已安装本应用条目。
pub fn is_installed() -> bool {
    read_root().map(|r| has_marker(&r)).unwrap_or(false)
}

/// 安装：写脚本（chmod 0755）+ 在 hooks.json 注册/更新条目。
pub fn install() -> Result<String> {
    let script = paths::cursor_hook_script();
    if let Some(dir) = script.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create script directory: {}", dir.display()))?;
    }
    atomic_write(&script, SCRIPT_CONTENT.as_bytes())
        .with_context(|| format!("failed to write script: {}", script.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .with_context(|| "failed to set script executable permission")?;
    }

    let root = read_root().unwrap_or_else(|| json!({ "version": 1, "hooks": {} }));
    let updated = upsert_entry(root, &script.to_string_lossy());
    write_root(&updated)?;

    let lang = crate::i18n::Lang::current();
    Ok(crate::i18n::tr(lang, "cmd.hookInstalled").to_string())
}

/// 移除：删除本应用条目 + 删除脚本文件（保留他人条目）。
pub fn uninstall() -> Result<String> {
    if let Some(root) = read_root() {
        let updated = remove_entries(root);
        write_root(&updated)?;
    }
    let script = paths::cursor_hook_script();
    if script.exists() {
        let _ = std::fs::remove_file(&script);
    }
    // 向后兼容：清理旧版脚本文件。
    let legacy = paths::legacy_cursor_hook_script();
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
    }
    let lang = crate::i18n::Lang::current();
    Ok(crate::i18n::tr(lang, "cmd.hookRemoved").to_string())
}

/// 在文件管理器中定位 hooks.json。
pub fn reveal() {
    let path = paths::cursor_hooks_json();
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        // 无“选中”语义，定位到所在目录。
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone());
        let _ = std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn();
    }
}

// MARK: - 纯函数（可测试）

fn entry_has_marker(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(|c| c.as_str())
        .map(|c| c.contains(MARKER) || c.contains(LEGACY_MARKER))
        .unwrap_or(false)
}

/// 任意 preToolUse 条目含 marker → 已安装。
pub fn has_marker(root: &Value) -> bool {
    root.get("hooks")
        .and_then(|h| h.get("preToolUse"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(entry_has_marker))
        .unwrap_or(false)
}

/// 插入或更新本应用条目，保留其他条目与未知字段。
pub fn upsert_entry(mut root: Value, script_path: &str) -> Value {
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let pre = hooks_obj.entry("preToolUse").or_insert_with(|| json!([]));
    if !pre.is_array() {
        *pre = json!([]);
    }
    let arr = pre.as_array_mut().unwrap();
    let entry = json!({ "command": script_path, "matcher": "Shell" });
    if let Some(slot) = arr.iter_mut().find(|e| entry_has_marker(e)) {
        *slot = entry;
    } else {
        arr.push(entry);
    }
    root
}

/// 移除本应用条目；若 preToolUse 变空则删除该键。
pub fn remove_entries(mut root: Value) -> Value {
    if let Some(obj) = root.as_object_mut() {
        if let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            let mut drop_key = false;
            if let Some(arr) = hooks.get_mut("preToolUse").and_then(|p| p.as_array_mut()) {
                arr.retain(|e| !entry_has_marker(e));
                drop_key = arr.is_empty();
            }
            if drop_key {
                hooks.remove("preToolUse");
            }
        }
    }
    root
}

// MARK: - 私有 IO

fn read_root() -> Option<Value> {
    let text = std::fs::read_to_string(paths::cursor_hooks_json()).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_root(root: &Value) -> Result<()> {
    let path = paths::cursor_hooks_json();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(root)?;
    atomic_write(&path, json.as_bytes())
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "/home/u/.cursor/hooks/askhuman-timeout.sh";
    const LEGACY_SCRIPT: &str = "/home/u/.cursor/hooks/humaninloop-timeout.sh";

    #[test]
    fn recognizes_legacy_marker() {
        // 旧版安装应被识别（向后兼容）：重装时替换、卸载时移除。
        let root = json!({
            "version": 1,
            "hooks": { "preToolUse": [ { "command": LEGACY_SCRIPT, "matcher": "Shell" } ] }
        });
        assert!(has_marker(&root));
        let upserted = upsert_entry(root, SCRIPT);
        let arr = upserted["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "旧条目应被替换而非新增");
        assert_eq!(arr[0]["command"], SCRIPT);
    }

    #[test]
    fn upsert_then_has_marker() {
        let root = upsert_entry(json!({ "version": 1, "hooks": {} }), SCRIPT);
        assert!(has_marker(&root));
        let arr = root["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], "Shell");
        assert_eq!(arr[0]["command"], SCRIPT);
    }

    #[test]
    fn upsert_is_idempotent() {
        let root = upsert_entry(json!({}), SCRIPT);
        let root = upsert_entry(root, SCRIPT);
        let arr = root["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "重复安装不应新增条目");
    }

    #[test]
    fn upsert_preserves_others() {
        let root = json!({
            "version": 1,
            "hooks": { "preToolUse": [ { "command": "other.sh", "matcher": "Shell" } ] }
        });
        let root = upsert_entry(root, SCRIPT);
        let arr = root["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(arr.iter().any(|e| e["command"] == "other.sh"));
        assert!(has_marker(&root));
    }

    #[test]
    fn remove_only_ours() {
        let root = json!({
            "version": 1,
            "hooks": { "preToolUse": [
                { "command": "other.sh", "matcher": "Shell" },
                { "command": SCRIPT, "matcher": "Shell" }
            ] }
        });
        let root = remove_entries(root);
        let arr = root["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["command"], "other.sh");
        assert!(!has_marker(&root));
    }

    #[test]
    fn remove_drops_empty_key() {
        let root = json!({
            "version": 1,
            "hooks": { "preToolUse": [ { "command": SCRIPT, "matcher": "Shell" } ] }
        });
        let root = remove_entries(root);
        assert!(root["hooks"].get("preToolUse").is_none(), "空数组应删除该键");
    }

    #[test]
    fn script_targets_askhuman() {
        assert!(SCRIPT_CONTENT.contains("AskHuman"));
        assert!(SCRIPT_CONTENT.contains("86400000"));
    }
}
