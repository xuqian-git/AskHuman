//! Slack Socket Mode 长连接（JSON 帧）。
//!
//! 协议：
//! 1. `POST https://slack.com/api/apps.connections.open`（App Token 走 `Authorization` 头）→ `url`（wss）。
//! 2. 连 wss；帧为 **JSON 文本**。业务帧 `events_api`（事件，如 `message`）/ `interactive`（交互，如
//!    `block_actions`），各含 `envelope_id`；另有 `hello`（建连）、`disconnect`（要求重连）控制帧。
//! 3. 每条含 `envelope_id` 的帧须 **3 秒内回 `{"envelope_id": id}`** ack。本实现**收帧即 ack**
//!    （与卡片更新解耦：卡片更新走 Web API `chat.update`，不绑 3 秒窗口），比飞书延迟回包更简单。
//! 4. WS 协议 Ping 回 Pong；`disconnect`/断开 → 重连（重新取 url）。

use super::SlackError;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 上抛给上层的业务事件（皆已 ack）。
pub enum WsEvent {
    /// 用户消息事件（`events_api` 的 `payload.event`，`type=message`）。
    Message(Value),
    /// 交互负载（`interactive` 的 `payload`，`type=block_actions`）。
    Interactive(Value),
}

pub struct SlackWs {
    http: reqwest::Client,
    app_token: String,
    write: SplitSink<Ws, Message>,
    read: SplitStream<Ws>,
}

impl SlackWs {
    /// 建立 Socket Mode 连接：取 wss url → 连 wss → 拆分读写。
    pub async fn connect(http: reqwest::Client, app_token: &str) -> Result<Self, SlackError> {
        let url = open_socket_url(&http, app_token).await?;
        let (ws, _resp) = connect_async(url)
            .await
            .map_err(|e| SlackError::Network(format!("WebSocket connection failed: {}", e)))?;
        let (write, read) = ws.split();
        Ok(Self {
            http,
            app_token: app_token.to_string(),
            write,
            read,
        })
    }

    /// 收下一个业务事件；内部处理 ack、hello/disconnect、ping/pong、断线重连。
    /// 返回 `None` 表示重连多次仍失败（上层据此结束）。
    pub async fn recv(&mut self) -> Option<WsEvent> {
        loop {
            let msg = self.read.next().await;
            match msg {
                Some(Ok(Message::Text(t))) => {
                    if let Some(ev) = self.handle_text(t.as_str()).await {
                        return Some(ev);
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    let _ = self.write.send(Message::Pong(p)).await;
                }
                Some(Ok(_)) => {} // Binary / Pong / 其它：忽略
                Some(Err(_)) | None => {
                    if !self.reconnect().await {
                        return None;
                    }
                }
            }
        }
    }

    /// 处理一条 JSON 文本帧；业务帧返回事件，控制帧返回 None。含 `envelope_id` 即先 ack。
    async fn handle_text(&mut self, text: &str) -> Option<WsEvent> {
        let v: Value = serde_json::from_str(text).ok()?;
        let frame_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        debug_log(&format!("[slack-ws] frame type={}", frame_type));

        // 控制帧：建连确认 / 要求重连。
        if frame_type == "hello" {
            return None;
        }
        if frame_type == "disconnect" {
            // 服务端要求重连（reason: warning / refresh_requested / too_many_connections）。
            if !self.reconnect().await {
                // 重连失败：交由 recv 的下一轮读到 None 后再尝试 / 退出。
            }
            return None;
        }

        // 业务帧：收帧即 ack（满足 3 秒，且与卡片更新解耦）。
        if let Some(id) = v.get("envelope_id").and_then(|e| e.as_str()) {
            self.ack(id).await;
        }

        match frame_type {
            "events_api" => {
                let event = v.get("payload").and_then(|p| p.get("event"))?;
                if event.get("type").and_then(|t| t.as_str()) != Some("message") {
                    return None;
                }
                // 跳过机器人自身消息与编辑/删除等噪音子类型（file_share 等保留）。
                if event.get("bot_id").is_some() {
                    return None;
                }
                if let Some(sub) = event.get("subtype").and_then(|s| s.as_str()) {
                    if matches!(
                        sub,
                        "bot_message" | "message_changed" | "message_deleted" | "message_replied"
                    ) {
                        return None;
                    }
                }
                Some(WsEvent::Message(event.clone()))
            }
            "interactive" => {
                let payload = v.get("payload")?;
                if payload.get("type").and_then(|t| t.as_str()) == Some("block_actions") {
                    Some(WsEvent::Interactive(payload.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// 回 ack：`{"envelope_id": id}`。
    async fn ack(&mut self, envelope_id: &str) {
        let body = json!({ "envelope_id": envelope_id }).to_string();
        let _ = self.write.send(Message::Text(body.into())).await;
    }

    /// 断线重连：重新取 url + 连接。最多重试若干次。
    async fn reconnect(&mut self) -> bool {
        for attempt in 0..5u32 {
            tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
            let Ok(url) = open_socket_url(&self.http, &self.app_token).await else {
                continue;
            };
            if let Ok((ws, _)) = connect_async(url).await {
                let (write, read) = ws.split();
                self.write = write;
                self.read = read;
                return true;
            }
        }
        false
    }
}

/// 取 Socket Mode wss URL：`apps.connections.open`（App Token 必须放 `Authorization` 头）。
pub async fn open_socket_url(
    http: &reqwest::Client,
    app_token: &str,
) -> Result<String, SlackError> {
    let resp = http
        .post(format!("{}/apps.connections.open", super::api_base()))
        .bearer_auth(app_token)
        .send()
        .await
        .map_err(|e| SlackError::Network(e.to_string()))?;
    let v: Value = resp.json().await.map_err(|_| SlackError::BadResponse)?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let msg = v
            .get("error")
            .and_then(|m| m.as_str())
            .unwrap_or("failed to open Socket Mode connection")
            .to_string();
        return Err(SlackError::Api(msg));
    }
    v.get("url")
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
        .ok_or(SlackError::BadResponse)
}

/// 是否开启 Slack 长连接诊断日志（环境变量 `ASKHUMAN_SLACK_DEBUG` 非空且非 "0"）。
pub fn debug_enabled() -> bool {
    std::env::var("ASKHUMAN_SLACK_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// 诊断日志：写入 `~/.askhuman/slack-debug.log`（GUI 模式 stderr 被静默，文件更可靠）。
pub fn debug_log(msg: &str) {
    if !debug_enabled() {
        return;
    }
    use std::io::Write;
    let dir = crate::paths::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("slack-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
    eprintln!("{}", msg);
}
