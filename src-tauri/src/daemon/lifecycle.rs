//! Daemon 生命周期支撑：二进制指纹、运行元信息（daemon.json）、单实例锁（flock）。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

/// 可执行文件指纹：用 mtime+size 判定「盘上二进制是否被换过」（dev 改逻辑但未 bump 版本时也能识别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub mtime_ms: u64,
    pub size: u64,
}

/// 计算当前可执行文件的指纹（解析失败回退到全 0）。
pub fn current_fingerprint() -> Fingerprint {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| {
            let size = m.len();
            let mtime_ms = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Fingerprint { mtime_ms, size }
        })
        .unwrap_or(Fingerprint {
            mtime_ms: 0,
            size: 0,
        })
}

/// 单实例锁文件 `~/.askhuman/daemon.lock`。
pub fn lock_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.lock")
}

/// 运行元信息文件 `~/.askhuman/daemon.json`。
pub fn meta_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.json")
}

/// 运行日志 `~/.askhuman/daemon.log`。
pub fn log_path() -> PathBuf {
    crate::paths::config_dir().join("daemon.log")
}

/// Daemon 运行元信息（落 daemon.json，供调试/排查）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonMeta {
    pub pid: u32,
    pub version: String,
    pub protocol_version: u32,
    pub started_at: u64,
    pub socket: String,
    pub fingerprint: Fingerprint,
}

pub fn write_meta(meta: &DaemonMeta) -> std::io::Result<()> {
    if let Some(dir) = meta_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_vec_pretty(meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(meta_path(), data)
}

/// 持有期间代表「本进程为唯一 Daemon」。Drop（文件关闭）时锁自动释放。
#[cfg(unix)]
pub struct LockGuard {
    _file: std::fs::File,
}

/// 尝试获取单实例锁（非阻塞）。
/// - `Ok(Some(guard))`：成功，本进程是唯一 Daemon。
/// - `Ok(None)`：已有其它 Daemon 持锁。
/// - `Err`：其它 IO 错误。
#[cfg(unix)]
pub fn acquire_lock() -> std::io::Result<Option<LockGuard>> {
    use std::os::unix::io::AsRawFd;
    if let Some(dir) = lock_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path())?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        // 已被其它进程持有（EWOULDBLOCK 与 EAGAIN 在各 Unix 上同值）。
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Ok(None);
        }
        return Err(err);
    }
    Ok(Some(LockGuard { _file: file }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_reflects_current_exe() {
        // 测试可执行文件存在 → size 非 0，且两次取值一致（盘上未变）。
        let a = current_fingerprint();
        let b = current_fingerprint();
        assert!(a.size > 0);
        assert_eq!(a, b);
    }

    #[test]
    fn meta_round_trip() {
        let meta = DaemonMeta {
            pid: 1,
            version: "9.9.9".into(),
            protocol_version: 1,
            started_at: 100,
            socket: "/tmp/x.sock".into(),
            fingerprint: Fingerprint {
                mtime_ms: 5,
                size: 6,
            },
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: DaemonMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pid, 1);
        assert_eq!(back.version, "9.9.9");
        assert_eq!(back.fingerprint.size, 6);
    }
}
