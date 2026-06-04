//! 钉钉 Stream 模式长连接：WebSocket 收机器人消息 + 卡片回调（零公网）。
//!
//! 流程：`POST /v1.0/gateway/connections/open` 拿 `endpoint`+`ticket` → 连 `endpoint?ticket=…` →
//! 循环收帧：SYSTEM ping 回 200；CALLBACK 先 3 秒内 ACK 再上抛事件；断线自动重连。

use super::DingTalkError;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub const TOPIC_BOT_MESSAGE: &str = "/v1.0/im/bot/messages/get";
pub const TOPIC_CARD_CALLBACK: &str = "/v1.0/card/instances/callback";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 上抛给上层的事件（`data` 为已解析的 JSON）。
pub enum StreamEvent {
    /// 机器人收到用户消息（文字/图片/文件/富文本）。
    BotMessage(Value),
    /// 卡片回调（按钮点选/发送）。
    CardCallback(Value),
}

pub struct StreamConn {
    http: reqwest::Client,
    client_id: String,
    client_secret: String,
    topics: Vec<String>,
    ws: Ws,
}

impl StreamConn {
    /// 建立连接并订阅 topics（CALLBACK）。
    pub async fn connect(
        http: reqwest::Client,
        client_id: &str,
        client_secret: &str,
        topics: &[&str],
    ) -> Result<Self, DingTalkError> {
        let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
        let ws = open_ws(&http, client_id, client_secret, &topics).await?;
        Ok(Self {
            http,
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            topics,
            ws,
        })
    }

    /// 收下一个业务事件；内部处理 SYSTEM ping / ACK / 断线重连。
    /// 返回 `None` 表示重连多次仍失败（上层据此结束）。
    pub async fn recv(&mut self) -> Option<StreamEvent> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(txt))) => {
                    if let Some(ev) = self.handle_frame(txt.as_str()).await {
                        return Some(ev);
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    let _ = self.ws.send(Message::Pong(p)).await;
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                    if !self.reconnect().await {
                        return None;
                    }
                }
                _ => {}
            }
        }
    }

    /// 处理一帧；业务帧返回事件，控制帧（ping/系统）返回 None。
    async fn handle_frame(&mut self, txt: &str) -> Option<StreamEvent> {
        let frame: Value = serde_json::from_str(txt).ok()?;
        let typ = frame.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let headers = frame.get("headers").cloned().unwrap_or(Value::Null);
        let topic = headers.get("topic").and_then(|t| t.as_str()).unwrap_or("");
        let message_id = headers
            .get("messageId")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let data: Value = frame
            .get("data")
            .and_then(|d| d.as_str())
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);

        match typ {
            "SYSTEM" => {
                // ping/KEEPALIVE 等：回 200 维持连接。
                self.ack(&message_id, json!({})).await;
                None
            }
            "CALLBACK" | "EVENT" => {
                // 先 ACK（3 秒内），再上抛。
                self.ack(&message_id, json!({})).await;
                match topic {
                    TOPIC_BOT_MESSAGE => Some(StreamEvent::BotMessage(data)),
                    TOPIC_CARD_CALLBACK => Some(StreamEvent::CardCallback(data)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 发送 ACK / 响应帧（带回原 messageId）。
    async fn ack(&mut self, message_id: &str, data: Value) {
        let frame = json!({
            "code": 200,
            "headers": { "contentType": "application/json", "messageId": message_id },
            "message": "OK",
            "data": data.to_string(),
        });
        let _ = self.ws.send(Message::Text(frame.to_string().into())).await;
    }

    /// 断线重连：重新 open 拿新 ticket 再连。最多重试若干次。
    async fn reconnect(&mut self) -> bool {
        for attempt in 0..5u32 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
            if let Ok(ws) =
                open_ws(&self.http, &self.client_id, &self.client_secret, &self.topics).await
            {
                self.ws = ws;
                return true;
            }
        }
        false
    }
}

/// 注册长连接 + 建 WebSocket。
async fn open_ws(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    topics: &[String],
) -> Result<Ws, DingTalkError> {
    let subscriptions: Vec<Value> = topics
        .iter()
        .map(|t| json!({ "type": "CALLBACK", "topic": t }))
        .collect();
    let body = json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "subscriptions": subscriptions,
        "ua": "askhuman/0.2",
        "localIp": "127.0.0.1",
    });
    let resp = http
        .post("https://api.dingtalk.com/v1.0/gateway/connections/open")
        .json(&body)
        .send()
        .await
        .map_err(|e| DingTalkError::Network(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|_| DingTalkError::BadResponse)?;
    if !status.is_success() {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("建立 Stream 连接失败")
            .to_string();
        return Err(DingTalkError::Api(msg));
    }
    let endpoint = v
        .get("endpoint")
        .and_then(|e| e.as_str())
        .ok_or(DingTalkError::BadResponse)?;
    let ticket = v
        .get("ticket")
        .and_then(|t| t.as_str())
        .ok_or(DingTalkError::BadResponse)?;
    let url = format!("{}?ticket={}", endpoint, ticket);
    let (ws, _resp) = connect_async(url)
        .await
        .map_err(|e| DingTalkError::Network(format!("WebSocket 连接失败: {}", e)))?;
    Ok(ws)
}
