//! 传输层：socket 路径 + 连接（客户端）/ 监听（Daemon）。
//!
//! Phase 0 实现 Unix domain socket（mac/Linux）。Windows named pipe 后续 Phase 补上。

use std::path::PathBuf;

/// IPC socket 路径 `~/.askhuman/daemon.sock`。
pub fn socket_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.sock")
}

#[cfg(unix)]
pub use unix_impl::{bind, connect};

#[cfg(unix)]
mod unix_impl {
    use super::socket_path;
    use tokio::net::{UnixListener, UnixStream};

    /// 客户端连接到 Daemon。
    pub async fn connect() -> std::io::Result<UnixStream> {
        UnixStream::connect(socket_path()).await
    }

    /// Daemon 绑定监听。调用前应已持有单实例锁（flock），故可安全删除残留 socket 再 bind。
    /// socket 权限设为 0600（仅本人）。
    pub fn bind() -> std::io::Result<UnixListener> {
        use std::os::unix::fs::PermissionsExt;
        let path = socket_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // 持锁前提下，任何已存在的 socket 都是残留，删之。
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        Ok(listener)
    }
}
