//! 进程间通信（IPC）：CLI / GUI Helper ↔ 常驻 Daemon。
//!
//! 传输：NDJSON（一行一个 JSON 消息）over Unix domain socket（mac/Linux）/ Windows named pipe。
//! 本文件定义协议消息类型；编解码见 `codec`，传输（socket 路径/连接/监听）见 `transport`。
//!
//! Phase 0 仅含握手与 daemon 控制（status/stop）；任务提交（submit/show/...）在后续 Phase 引入。

pub mod codec;
pub mod transport;

pub use codec::{read_msg, write_msg};

use crate::daemon::lifecycle::Fingerprint;
use serde::{Deserialize, Serialize};

/// IPC 协议版本：不兼容变更时 +1，握手不一致即触发换新。
pub const PROTOCOL_VERSION: u32 = 1;

/// CLI/GUI 连接时的握手信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientHello {
    pub protocol_version: u32,
    pub client_version: String,
    pub binary_path: String,
    pub fingerprint: Fingerprint,
    pub pid: u32,
}

/// 握手结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelloStatus {
    /// 正常，可继续。
    Ok,
    /// Daemon 已过时（二进制指纹/协议变化），将自行退出；客户端应等其下线后用新二进制拉起。
    Restarting,
}

/// 对 `ClientHello` 的回应。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloAck {
    pub protocol_version: u32,
    pub daemon_version: String,
    pub status: HelloStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `daemon status` 返回的运行信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub pid: u32,
    pub version: String,
    pub protocol_version: u32,
    pub uptime_secs: u64,
    pub socket: String,
    pub active_requests: usize,
}

/// 客户端 → Daemon 的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMsg {
    Hello(ClientHello),
    Status,
    Stop,
}

/// Daemon → 客户端的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMsg {
    HelloAck(HelloAck),
    Status(StatusInfo),
    Stopping,
    Error { message: String },
}
