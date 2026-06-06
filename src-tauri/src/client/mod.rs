//! CLI 作为 Daemon 客户端：连接 / 握手 / 按需自启 / 换新重连，以及 status·stop 辅助。
//!
//! Phase 0：仅提供连通性原语（ensure_running / request_status / request_stop）。
//! 任务提交（submit）将在 Phase 1 接入。

use crate::daemon::lifecycle;
use crate::daemon::spawn;
use crate::ipc::{self, transport, ClientHello, ClientMsg, HelloStatus, ServerMsg, StatusInfo};
use std::io::{Error, ErrorKind};
use std::time::{Duration, Instant};
use tokio::io::BufReader;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

type Reader = BufReader<OwnedReadHalf>;

/// 本进程的握手信息。
fn hello() -> ClientHello {
    ClientHello {
        protocol_version: ipc::PROTOCOL_VERSION,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        binary_path: std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        fingerprint: lifecycle::current_fingerprint(),
        pid: std::process::id(),
    }
}

/// 连接并拆分读写半。
async fn connect_split() -> std::io::Result<(Reader, OwnedWriteHalf)> {
    let stream = transport::connect().await?;
    let (r, w) = stream.into_split();
    Ok((BufReader::new(r), w))
}

/// 连一次并握手，返回握手状态（连不上返回 None）。
async fn hello_status() -> Option<HelloStatus> {
    let (mut reader, mut writer) = connect_split().await.ok()?;
    ipc::write_msg(&mut writer, &ClientMsg::Hello(hello())).await.ok()?;
    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::HelloAck(ack))) => Some(ack.status),
        _ => None,
    }
}

/// 确保 Daemon 在运行且为当前二进制版本；必要时自启 / 等旧实例换新。
pub async fn ensure_running() -> std::io::Result<()> {
    // 1. 已在运行且 Ok → 直接用。
    match hello_status().await {
        Some(HelloStatus::Ok) => return Ok(()),
        Some(HelloStatus::Restarting) => {
            // 旧实例将自行退出；等它下线后再拉起新的。
            wait_until_down(Duration::from_secs(5)).await;
        }
        None => {}
    }

    // 2. 拉起并等待就绪（最多约 5 秒）。
    spawn::spawn_detached()?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Some(HelloStatus::Ok) = hello_status().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(Error::new(
        ErrorKind::TimedOut,
        "daemon did not become ready in time",
    ))
}

/// 请求运行状态（未运行返回 None）。
pub async fn request_status() -> Option<StatusInfo> {
    let (mut reader, mut writer) = connect_split().await.ok()?;
    ipc::write_msg(&mut writer, &ClientMsg::Status).await.ok()?;
    match ipc::read_msg::<_, ServerMsg>(&mut reader).await {
        Ok(Some(ServerMsg::Status(info))) => Some(info),
        _ => None,
    }
}

/// 请求停止；收到 Stopping 回应返回 true，未运行返回 false。
pub async fn request_stop() -> bool {
    let Ok((mut reader, mut writer)) = connect_split().await else {
        return false;
    };
    if ipc::write_msg(&mut writer, &ClientMsg::Stop).await.is_err() {
        return false;
    }
    matches!(
        ipc::read_msg::<_, ServerMsg>(&mut reader).await,
        Ok(Some(ServerMsg::Stopping))
    )
}

/// 轮询直到 Daemon 不可连（或超时）。
pub async fn wait_until_down(max: Duration) {
    let start = Instant::now();
    while start.elapsed() < max {
        if transport::connect().await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
