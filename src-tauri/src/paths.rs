//! 跨平台路径：配置目录、临时目录、Cursor 相关路径。

use std::path::PathBuf;

/// 用户主目录（解析失败时回退到当前目录，保证不 panic）。
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 配置目录 `~/.humaninloop`。
pub fn config_dir() -> PathBuf {
    home().join(".humaninloop")
}

/// 配置文件 `~/.humaninloop/config.json`。
pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

/// 本次请求的图片落盘目录 `temp/humaninloop/<request_id>/`。
pub fn request_temp_dir(request_id: &str) -> PathBuf {
    std::env::temp_dir().join("humaninloop").join(request_id)
}

/// Cursor 目录 `~/.cursor`。
pub fn cursor_dir() -> PathBuf {
    home().join(".cursor")
}

/// `~/.cursor/hooks.json`。
pub fn cursor_hooks_json() -> PathBuf {
    cursor_dir().join("hooks.json")
}

/// `~/.cursor/hooks/humaninloop-timeout.sh`。
pub fn cursor_hook_script() -> PathBuf {
    cursor_dir().join("hooks").join("humaninloop-timeout.sh")
}
