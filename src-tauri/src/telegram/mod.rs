//! Telegram Bot API 最小客户端（当前用于设置页「测试连接」）。
//! 完整会话/长轮询 Channel 在 Step 7 实现。

use serde_json::json;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum TelegramError {
    EmptyToken,
    EmptyChatId,
    InvalidChatId,
    Api(String),
    Network(String),
    BadResponse,
}

impl fmt::Display for TelegramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TelegramError::EmptyToken => write!(f, "Bot Token 不能为空"),
            TelegramError::EmptyChatId => write!(f, "Chat ID 不能为空"),
            TelegramError::InvalidChatId => write!(f, "Chat ID 格式无效，请输入有效的数字 ID"),
            TelegramError::Api(msg) => write!(f, "Telegram API 错误: {}", msg),
            TelegramError::Network(msg) => write!(f, "网络错误: {}", msg),
            TelegramError::BadResponse => write!(f, "无法解析 Telegram 响应"),
        }
    }
}

impl std::error::Error for TelegramError {}

pub struct TelegramClient {
    token: String,
    chat_id: i64,
    api_base_url: String,
}

impl TelegramClient {
    pub fn new(token: String, chat_id_string: String, api_base_url: String) -> Result<Self, TelegramError> {
        let token = token.trim().to_string();
        let chat = chat_id_string.trim().to_string();
        if token.is_empty() {
            return Err(TelegramError::EmptyToken);
        }
        if chat.is_empty() {
            return Err(TelegramError::EmptyChatId);
        }
        if chat.starts_with('@') {
            return Err(TelegramError::InvalidChatId);
        }
        let chat_id: i64 = chat.parse().map_err(|_| TelegramError::InvalidChatId)?;
        let base = api_base_url.trim();
        let api_base_url = if base.is_empty() {
            "https://api.telegram.org".to_string()
        } else {
            base.to_string()
        };
        Ok(Self {
            token,
            chat_id,
            api_base_url,
        })
    }

    /// 发送消息，返回 message_id。
    pub async fn send_message(&self, text: &str) -> Result<i64, TelegramError> {
        let url = format!("{}/bot{}/sendMessage", self.api_base_url, self.token);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| TelegramError::Network(e.to_string()))?;
        let body = json!({ "chat_id": self.chat_id, "text": text });
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| TelegramError::Network(e.to_string()))?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|_| TelegramError::BadResponse)?;
        if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(v.get("result")
                .and_then(|r| r.get("message_id"))
                .and_then(|m| m.as_i64())
                .unwrap_or(0))
        } else {
            let desc = v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("sendMessage 失败")
                .to_string();
            Err(TelegramError::Api(desc))
        }
    }

    /// 发送测试消息验证配置。
    pub async fn test_connection(&self) -> Result<String, TelegramError> {
        let text = "🤖 HumanInLoop 测试消息\n\n这是一条测试消息，表示 Telegram Bot 配置成功！";
        self.send_message(text).await?;
        Ok("测试消息发送成功！Telegram Bot 配置正确。".to_string())
    }
}
