//! Claude Code Hook 集成：安装/移除/状态 + 跨平台打开 settings.json。
//!
//! 作用与 Cursor Hook 对称：在 `~/.claude/settings.json` 注册一条 `PreToolUse`（matcher=`Bash`）
//! 命令钩子，检测到 Shell 调用 AskHuman 时把该次工具调用的 `timeout` 提升到 24 小时；同时把
//! `env.BASH_MAX_TIMEOUT_MS` 抬高到 24 小时，否则钩子设置的超时会被 Claude 默认上限（10 分钟）钳掉。
//!
//! settings.json 的增删为纯函数，便于单测（幂等、保留用户其它条目）。卸载只移除本应用注入的
//! `PreToolUse` 条目与脚本文件，**不动 `env`**（用户可能依赖该上限）。

use crate::paths;
use anyhow::{Context, Result};
use serde_json::{json, Value};

/// 识别本应用条目的标记（脚本文件名）。
pub const MARKER: &str = "askhuman-timeout.sh";

/// 抬高的 Bash 超时上限环境变量名。
pub const BASH_MAX_KEY: &str = "BASH_MAX_TIMEOUT_MS";

/// 目标超时上限（24 小时，毫秒），与脚本写入的 timeout 一致。
pub const BASH_MAX_MS: u64 = 86_400_000;

/// 钩子脚本内容（安装时写入并 chmod 0755）。
///
/// 读取 stdin 的 `tool_input`，命中 AskHuman 调用时返回 `permissionDecision:allow` + `updatedInput`
/// （把原 `tool_input` 原样回写、仅覆盖 `timeout`，规避 Claude「整体替换」语义丢字段）；未命中或
/// 解析失败时输出 `{}`，保持 fail-open。AskHuman 匹配正则与 Cursor 脚本逐字一致。
pub const SCRIPT_CONTENT: &str = r##"#!/usr/bin/env bash
# askhuman-timeout.sh
# 由 AskHuman 自动安装 / 移除，请勿手动编辑。
#
# 作用：作为 Claude Code 的 PreToolUse 钩子，检测 Bash 工具调用是否会执行 AskHuman
# 命令；命中时把该次工具调用 timeout 提升至 24 小时（86400000ms），防止等待用户回应
# 时被强制取消。未命中或解析失败时输出空对象 `{}`，保持 fail-open。

set -u

# 从 stdin 读取 JSON 输入
input=$(cat)

# 解析 .tool_input.command，按 python3 -> jq 顺序回退
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
fi

# 匹配 AskHuman 调用：兼顾行内任意位置 / 链式命令 / 引号包裹 / 绝对路径前缀
if [ -n "$command" ] && printf '%s' "$command" \
     | grep -Eq "(^|[[:space:];&|()\`\"'\\]|/)AskHuman([[:space:]]|$|[\"'\\])"; then
  # 命中：把原 tool_input 原样回写并覆盖 timeout
  if command -v python3 >/dev/null 2>&1; then
    output=$(printf '%s' "$input" | python3 -c '
import json
import sys

try:
    data = json.load(sys.stdin)
    ti = data.get("tool_input") or {}
    if not isinstance(ti, dict):
        ti = {}
    ti["timeout"] = 86400000
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow", "updatedInput": ti}}))
except Exception:
    print("{}")
')
  elif command -v jq >/dev/null 2>&1; then
    output=$(printf '%s' "$input" | jq -c '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"allow",updatedInput:((.tool_input // {}) + {timeout:86400000})}}' 2>/dev/null)
    [ -z "$output" ] && output='{}'
  else
    output='{}'
  fi
else
  output='{}'
fi

printf '%s\n' "$output"

exit 0
"##;

/// 当前平台是否支持（脚本为 bash，仅 unix：macOS/Linux；Windows 不支持）。
pub fn supported() -> bool {
    cfg!(unix)
}

/// settings.json 是否存在。
pub fn settings_exists() -> bool {
    paths::claude_settings_json().exists()
}

/// 是否已安装本应用条目。
pub fn is_installed() -> bool {
    read_root().map(|r| has_marker(&r)).unwrap_or(false)
}

/// 已安装但磁盘脚本与内置脚本不一致（或脚本缺失）→ 需更新。
pub fn needs_update() -> bool {
    if !is_installed() {
        return false;
    }
    match std::fs::read_to_string(paths::claude_hook_script()) {
        Ok(content) => content != SCRIPT_CONTENT,
        Err(_) => true,
    }
}

/// 安装：写脚本（chmod 0755）+ 注册/更新 PreToolUse 条目 + 抬高 BASH_MAX_TIMEOUT_MS。
pub fn install() -> Result<String> {
    let script = paths::claude_hook_script();
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

    let root = read_root().unwrap_or_else(|| json!({}));
    let root = upsert_hook(root, &script.to_string_lossy());
    let root = ensure_env_max(root);
    write_root(&root)?;

    let lang = crate::i18n::Lang::current();
    Ok(crate::i18n::tr(lang, "cmd.hookInstalled").to_string())
}

/// 更新：用最新脚本与条目覆盖（复用 install），同时确保 env 上限 ≥ 24h。
pub fn update() -> Result<String> {
    install()?;
    let lang = crate::i18n::Lang::current();
    Ok(crate::i18n::tr(lang, "cmd.hookUpdated").to_string())
}

/// 移除：删除本应用 PreToolUse 条目 + 删除脚本文件；保留 `env` 与其它条目。
pub fn uninstall() -> Result<String> {
    if let Some(root) = read_root() {
        let updated = remove_hook(root);
        write_root(&updated)?;
    }
    let script = paths::claude_hook_script();
    if script.exists() {
        let _ = std::fs::remove_file(&script);
    }
    let lang = crate::i18n::Lang::current();
    Ok(crate::i18n::tr(lang, "cmd.hookRemoved").to_string())
}

/// 在文件管理器中定位 settings.json。
pub fn reveal() {
    let path = paths::claude_settings_json();
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone());
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn();
    }
}

// MARK: - 纯函数（可测试）

fn group_has_marker(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter().any(|entry| {
                entry
                    .get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| c.contains(MARKER))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 任意 PreToolUse matcher 组含本应用脚本 → 已安装。
pub fn has_marker(root: &Value) -> bool {
    root.get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(group_has_marker))
        .unwrap_or(false)
}

/// 插入或更新本应用 PreToolUse 条目（matcher=Bash），保留其他条目与未知字段。
pub fn upsert_hook(mut root: Value, script_path: &str) -> Value {
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let pre = hooks_obj.entry("PreToolUse").or_insert_with(|| json!([]));
    if !pre.is_array() {
        *pre = json!([]);
    }
    let arr = pre.as_array_mut().unwrap();
    let group = json!({
        "matcher": "Bash",
        "hooks": [ { "type": "command", "command": script_path } ]
    });
    if let Some(slot) = arr.iter_mut().find(|g| group_has_marker(g)) {
        *slot = group;
    } else {
        arr.push(group);
    }
    root
}

/// 确保 `env.BASH_MAX_TIMEOUT_MS` ≥ 24h：缺失或现值更小时设为 24h；现值更大则保留。
pub fn ensure_env_max(mut root: Value) -> Value {
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let env = obj.entry("env").or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env_obj = env.as_object_mut().unwrap();
    let current = env_obj.get(BASH_MAX_KEY).and_then(parse_ms);
    if current.map(|v| v < BASH_MAX_MS).unwrap_or(true) {
        env_obj.insert(BASH_MAX_KEY.to_string(), json!(BASH_MAX_MS.to_string()));
    }
    root
}

/// 解析环境变量值（字符串或数字）为毫秒数。
fn parse_ms(v: &Value) -> Option<u64> {
    match v {
        Value::String(s) => s.trim().parse::<u64>().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

/// 移除本应用 PreToolUse 条目；若数组 / hooks 变空则删除对应键。不动 `env`。
pub fn remove_hook(mut root: Value) -> Value {
    if let Some(obj) = root.as_object_mut() {
        if let Some(hooks) = obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            let mut drop_pre = false;
            if let Some(arr) = hooks.get_mut("PreToolUse").and_then(|p| p.as_array_mut()) {
                arr.retain(|g| !group_has_marker(g));
                drop_pre = arr.is_empty();
            }
            if drop_pre {
                hooks.remove("PreToolUse");
            }
            let hooks_empty = hooks.is_empty();
            if hooks_empty {
                obj.remove("hooks");
            }
        }
    }
    root
}

// MARK: - 私有 IO

fn read_root() -> Option<Value> {
    let text = std::fs::read_to_string(paths::claude_settings_json()).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_root(root: &Value) -> Result<()> {
    let path = paths::claude_settings_json();
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

    const SCRIPT: &str = "/home/u/.claude/hooks/askhuman-timeout.sh";

    #[test]
    fn upsert_then_has_marker() {
        let root = upsert_hook(json!({}), SCRIPT);
        assert!(has_marker(&root));
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], "Bash");
        assert_eq!(arr[0]["hooks"][0]["command"], SCRIPT);
        assert_eq!(arr[0]["hooks"][0]["type"], "command");
    }

    #[test]
    fn upsert_is_idempotent() {
        let root = upsert_hook(json!({}), SCRIPT);
        let root = upsert_hook(root, SCRIPT);
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "重复安装不应新增条目");
    }

    #[test]
    fn upsert_preserves_other_groups() {
        let root = json!({
            "hooks": { "PreToolUse": [ { "matcher": "Edit", "hooks": [ { "type": "command", "command": "other.sh" } ] } ] }
        });
        let root = upsert_hook(root, SCRIPT);
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(arr.iter().any(|g| g["hooks"][0]["command"] == "other.sh"));
        assert!(has_marker(&root));
    }

    #[test]
    fn ensure_env_sets_when_absent() {
        let root = ensure_env_max(json!({}));
        assert_eq!(root["env"][BASH_MAX_KEY], json!(BASH_MAX_MS.to_string()));
    }

    #[test]
    fn ensure_env_keeps_larger_value() {
        let bigger = (BASH_MAX_MS + 1).to_string();
        let root = ensure_env_max(json!({ "env": { BASH_MAX_KEY: bigger.clone() } }));
        assert_eq!(root["env"][BASH_MAX_KEY], json!(bigger));
    }

    #[test]
    fn ensure_env_raises_smaller_value() {
        let root = ensure_env_max(json!({ "env": { BASH_MAX_KEY: "600000" } }));
        assert_eq!(root["env"][BASH_MAX_KEY], json!(BASH_MAX_MS.to_string()));
    }

    #[test]
    fn remove_only_ours_and_keeps_env() {
        let root = json!({
            "env": { BASH_MAX_KEY: BASH_MAX_MS.to_string(), "FOO": "bar" },
            "hooks": { "PreToolUse": [
                { "matcher": "Edit", "hooks": [ { "type": "command", "command": "other.sh" } ] },
                { "matcher": "Bash", "hooks": [ { "type": "command", "command": SCRIPT } ] }
            ] }
        });
        let root = remove_hook(root);
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["hooks"][0]["command"], "other.sh");
        assert!(!has_marker(&root));
        // env 保持不动
        assert_eq!(root["env"][BASH_MAX_KEY], json!(BASH_MAX_MS.to_string()));
        assert_eq!(root["env"]["FOO"], "bar");
    }

    #[test]
    fn remove_drops_empty_keys() {
        let root = json!({
            "hooks": { "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": SCRIPT } ] } ] }
        });
        let root = remove_hook(root);
        assert!(root.get("hooks").is_none(), "空 hooks 应整体删除");
    }

    #[test]
    fn script_targets_askhuman() {
        assert!(SCRIPT_CONTENT.contains("AskHuman"));
        assert!(SCRIPT_CONTENT.contains("86400000"));
        assert!(SCRIPT_CONTENT.contains("permissionDecision"));
    }
}
