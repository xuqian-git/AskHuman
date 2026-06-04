//! 钉钉单聊机器人 OpenAPI 客户端（reqwest）。
//!
//! robotCode 统一取 `client_id`（企业内部应用机器人 robotCode = AppKey）。

use super::token;
use super::DingTalkError;
use crate::config::DingTalkChannelConfig;
use serde_json::{json, Value};
use std::time::Duration;

const API_BASE: &str = "https://api.dingtalk.com";
const OAPI_BASE: &str = "https://oapi.dingtalk.com";

pub struct DingTalkClient {
    client_id: String,
    client_secret: String,
    user_id: String,
    http: reqwest::Client,
}

impl DingTalkClient {
    pub fn new(config: &DingTalkChannelConfig) -> Result<Self, DingTalkError> {
        let client_id = config.client_id.trim().to_string();
        let client_secret = config.client_secret.trim().to_string();
        let user_id = config.user_id.trim().to_string();
        if client_id.is_empty() {
            return Err(DingTalkError::EmptyConfig("ClientId".into()));
        }
        if client_secret.is_empty() {
            return Err(DingTalkError::EmptyConfig("ClientSecret".into()));
        }
        if user_id.is_empty() {
            return Err(DingTalkError::EmptyConfig("UserId".into()));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| DingTalkError::Network(e.to_string()))?;
        Ok(Self {
            client_id,
            client_secret,
            user_id,
            http,
        })
    }

    /// 机器人编码 = AppKey(ClientId)。
    pub fn robot_code(&self) -> &str {
        &self.client_id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn token(&self) -> Result<String, DingTalkError> {
        token::get_token(&self.http, &self.client_id, &self.client_secret).await
    }

    /// 调用新版接口（`https://api.dingtalk.com{path}`，header 携带 access_token），返回响应体。
    /// 以 HTTP 2xx 判定成功；失败时取 body.message 作为错误信息。
    pub(crate) async fn call_new(&self, path: &str, body: Value) -> Result<Value, DingTalkError> {
        let token = self.token().await?;
        let resp = self
            .http
            .post(format!("{}{}", API_BASE, path))
            .header("x-acs-dingtalk-access-token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(v)
        } else {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("请求失败")
                .to_string();
            Err(DingTalkError::Api(msg))
        }
    }

    // ===== 单聊主动发送（oToMessages/batchSend）=====

    async fn send_oto(&self, msg_key: &str, msg_param: Value) -> Result<(), DingTalkError> {
        let body = json!({
            "robotCode": self.robot_code(),
            "userIds": [self.user_id],
            "msgKey": msg_key,
            "msgParam": msg_param.to_string(),
        });
        self.call_new("/v1.0/robot/oToMessages/batchSend", body).await?;
        Ok(())
    }

    pub async fn send_oto_text(&self, content: &str) -> Result<(), DingTalkError> {
        self.send_oto("sampleText", json!({ "content": content })).await
    }

    pub async fn send_oto_markdown(&self, title: &str, text: &str) -> Result<(), DingTalkError> {
        self.send_oto("sampleMarkdown", json!({ "title": title, "text": text }))
            .await
    }

    pub async fn send_oto_image(&self, media_id: &str) -> Result<(), DingTalkError> {
        self.send_oto("sampleImageMsg", json!({ "photoURL": media_id }))
            .await
    }

    pub async fn send_oto_file(
        &self,
        media_id: &str,
        file_name: &str,
        file_type: &str,
    ) -> Result<(), DingTalkError> {
        self.send_oto(
            "sampleFile",
            json!({ "mediaId": media_id, "fileName": file_name, "fileType": file_type }),
        )
        .await
    }

    // ===== 媒体上传（旧 oapi，access_token 走 query）=====

    /// 上传媒体文件，返回 media_id。`kind` 为 `image` / `file`。
    pub async fn upload_media(&self, path: &str, kind: &str) -> Result<String, DingTalkError> {
        let token = self.token().await?;
        let bytes = std::fs::read(path)
            .map_err(|e| DingTalkError::Network(format!("读取文件失败: {}", e)))?;
        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name);
        let form = reqwest::multipart::Form::new().part("media", part);
        let url = format!("{}/media/upload?access_token={}&type={}", OAPI_BASE, token, kind);
        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|_| DingTalkError::BadResponse)?;
        if v.get("errcode").and_then(|c| c.as_i64()) == Some(0) {
            v.get("media_id")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
                .ok_or(DingTalkError::BadResponse)
        } else {
            let msg = v
                .get("errmsg")
                .and_then(|m| m.as_str())
                .unwrap_or("媒体上传失败")
                .to_string();
            Err(DingTalkError::Api(msg))
        }
    }

    // ===== 互动卡片（保留给高级版卡片 A 方案，B 方案暂不使用）=====

    /// 发送题目互动卡片到单聊。`biz_id` 同时作为 cardBizId / outTrackId（幂等 + 回调定位）。
    #[allow(dead_code)]
    pub async fn send_card(&self, biz_id: &str, card_data: &str) -> Result<(), DingTalkError> {
        let receiver = json!({ "userId": self.user_id }).to_string();
        let body = json!({
            "cardTemplateId": "StandardCard",
            "cardBizId": biz_id,
            "robotCode": self.robot_code(),
            "singleChatReceiver": receiver,
            "cardData": card_data,
            "callbackType": "STREAM",
        });
        self.call_new("/v1.0/im/v1.0/robot/interactiveCards/send", body)
            .await?;
        Ok(())
    }

    /// 更新已发出的卡片（点选后刷新高亮）。
    #[allow(dead_code)]
    pub async fn update_card(&self, biz_id: &str, card_data: &str) -> Result<(), DingTalkError> {
        let body = json!({
            "cardBizId": biz_id,
            "cardData": card_data,
            "robotCode": self.robot_code(),
        });
        self.call_new("/v1.0/im/v1.0/robot/interactiveCards", body)
            .await?;
        Ok(())
    }

    // ===== 接收消息中的文件下载 =====

    /// 用 downloadCode 换临时下载链接并下载到本地临时文件，返回本地路径。
    /// `ext` 为期望扩展名（钉钉下载默认 `.file`，需按真实类型修正）。
    pub async fn download_message_file_to(
        &self,
        download_code: &str,
        ext: &str,
    ) -> Result<String, DingTalkError> {
        let body = json!({ "downloadCode": download_code, "robotCode": self.robot_code() });
        let v = self
            .call_new("/v1.0/robot/messageFiles/download", body)
            .await?;
        let url = v
            .get("downloadUrl")
            .and_then(|u| u.as_str())
            .ok_or(DingTalkError::BadResponse)?;
        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| DingTalkError::Network(e.to_string()))?;

        let dir = std::env::temp_dir().join("askhuman-dingtalk");
        std::fs::create_dir_all(&dir)
            .map_err(|e| DingTalkError::Network(format!("创建临时目录失败: {}", e)))?;
        let ext = ext.trim_start_matches('.');
        let name = if ext.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            format!("{}.{}", uuid::Uuid::new_v4(), ext)
        };
        let dest = dir.join(name);
        std::fs::write(&dest, &bytes)
            .map_err(|e| DingTalkError::Network(format!("写入临时文件失败: {}", e)))?;
        Ok(dest.to_string_lossy().to_string())
    }
}
