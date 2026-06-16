//! 跨平台路径：配置目录、临时目录、Cursor 相关路径。

use std::path::PathBuf;

/// 用户主目录（解析失败时回退到当前目录，保证不 panic）。
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 配置目录 `~/.askhuman`（写入与读取的规范位置）。
pub fn config_dir() -> PathBuf {
    home().join(".askhuman")
}

/// 旧版配置目录 `~/.humaninloop`（仅用于向后兼容读取）。
pub fn legacy_config_dir() -> PathBuf {
    home().join(".humaninloop")
}

/// 配置文件 `~/.askhuman/config.json`。
pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// 旧版配置文件 `~/.humaninloop/config.json`（仅用于向后兼容读取）。
pub fn legacy_config_file() -> PathBuf {
    legacy_config_dir().join("config.json")
}

/// 本次请求的图片落盘目录 `temp/askhuman/<request_id>/`。
pub fn request_temp_dir(request_id: &str) -> PathBuf {
    std::env::temp_dir().join("askhuman").join(request_id)
}

/// 回复历史文件 `~/.askhuman/history.jsonl`（每行一条 JSON）。
pub fn history_file() -> PathBuf {
    config_dir().join("history.jsonl")
}

/// 回复历史写入锁 `~/.askhuman/history.lock`（写/裁剪/清空时持有）。
pub fn history_lock() -> PathBuf {
    config_dir().join("history.lock")
}

/// 版本自更新状态文件 `~/.askhuman/update.json`（最新版本/检查时间/忽略集合/待生效）。
pub fn update_state_file() -> PathBuf {
    config_dir().join("update.json")
}

/// Agent 生命周期追踪状态文件 `~/.askhuman/agents.json`（daemon 持久化、重启复核用）。
pub fn agents_file() -> PathBuf {
    config_dir().join("agents.json")
}

/// 杂项运行时状态目录 `~/.askhuman/state`（daemon 跨重启保留的小状态）。
pub fn state_dir() -> PathBuf {
    config_dir().join("state")
}

/// 「IM 会话期自动激活」当前活跃槽持久化文件 `~/.askhuman/state/auto-channel.json`。
/// 跨 daemon 重启保留；仅由「用户在某渠道的入站消息」更新。
pub fn auto_channel_file() -> PathBuf {
    state_dir().join("auto-channel.json")
}

/// GUI 宿主进程的 IPC socket `~/.askhuman/gui-host.sock`（与 daemon socket 解耦，
/// 使 daemon 未运行时也能打开设置/历史窗口，spec D13）。
pub fn gui_host_sock() -> PathBuf {
    config_dir().join("gui-host.sock")
}

/// GUI 宿主进程的单实例锁 `~/.askhuman/gui-host.lock`（flock，保证全局唯一宿主）。
pub fn gui_host_lock() -> PathBuf {
    config_dir().join("gui-host.lock")
}

/// Cursor 目录 `~/.cursor`。
pub fn cursor_dir() -> PathBuf {
    home().join(".cursor")
}

/// `~/.cursor/hooks.json`。
pub fn cursor_hooks_json() -> PathBuf {
    cursor_dir().join("hooks.json")
}

/// `~/.cursor/hooks/askhuman-timeout.sh`。
pub fn cursor_hook_script() -> PathBuf {
    cursor_dir().join("hooks").join("askhuman-timeout.sh")
}

/// 旧版 hook 脚本 `~/.cursor/hooks/humaninloop-timeout.sh`（仅用于向后兼容清理）。
pub fn legacy_cursor_hook_script() -> PathBuf {
    cursor_dir().join("hooks").join("humaninloop-timeout.sh")
}

/// Cursor 全局规则目录 `~/.cursor/rules`（用户级，跨项目，文件为 `*.mdc`）。
pub fn cursor_rules_dir() -> PathBuf {
    cursor_dir().join("rules")
}

/// 本应用独占的 Cursor 全局规则文件 `~/.cursor/rules/askhuman.mdc`。
pub fn cursor_rule_file() -> PathBuf {
    cursor_rules_dir().join("askhuman.mdc")
}

/// Cursor 用户级 MCP 配置文件 `~/.cursor/mcp.json`（与 hooks.json 不同文件）。
pub fn cursor_mcp_json() -> PathBuf {
    cursor_dir().join("mcp.json")
}

/// Claude Code 配置目录 `~/.claude`。
pub fn claude_dir() -> PathBuf {
    home().join(".claude")
}

/// Claude Code 全局 memory 文件 `~/.claude/CLAUDE.md`（用户级，跨项目）。
pub fn claude_md() -> PathBuf {
    claude_dir().join("CLAUDE.md")
}

/// Claude Code 用户级设置文件 `~/.claude/settings.json`（hooks / env 等都写在此）。
pub fn claude_settings_json() -> PathBuf {
    claude_dir().join("settings.json")
}

/// Claude Code 用户级主配置文件 `~/.claude.json`（top-level `mcpServers` 写于此；
/// 文件通常很大、含大量项目历史，必须最小化编辑、绝不整写）。
pub fn claude_json() -> PathBuf {
    home().join(".claude.json")
}

/// Claude Code hook 脚本 `~/.claude/hooks/askhuman-timeout.sh`。
pub fn claude_hook_script() -> PathBuf {
    claude_dir().join("hooks").join("askhuman-timeout.sh")
}

/// Codex 配置目录 `~/.codex`。
pub fn codex_dir() -> PathBuf {
    home().join(".codex")
}

/// Codex 全局指令文件 `~/.codex/AGENTS.md`（用户级，跨项目）。
pub fn codex_agents_md() -> PathBuf {
    codex_dir().join("AGENTS.md")
}

/// Codex 用户级配置文件 `~/.codex/config.toml`（生命周期 hook 的信任 `[hooks.state]` 写于此）。
pub fn codex_config_toml() -> PathBuf {
    codex_dir().join("config.toml")
}

/// Codex 用户级 hook 定义文件 `~/.codex/hooks.json`（生命周期 hook 写于此；信任键以其绝对路径为前缀）。
pub fn codex_hooks_json() -> PathBuf {
    codex_dir().join("hooks.json")
}
