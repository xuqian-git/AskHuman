//! 钉钉 OpenAPI / Stream 客户端层。
//!
//! 形态：企业内部应用 + 机器人 + Stream 模式 + 单聊。
//! - 鉴权：`token`（access_token 缓存，新旧接口同一 token）。
//! - 发送：`client`（单聊文本/图片/文件、媒体上传、消息文件下载、互动卡片发送/更新）。
//! - 卡片：`card`（StandardCard cardData 构造 + 回调解析）。
//! - 接收：`stream`（Stream 长连接：bot 消息 + 卡片回调）。
//!
//! robotCode 不单独配置：企业内部应用机器人 robotCode = clientId(AppKey)。

pub mod card;
pub mod client;
pub mod stream;
pub mod token;

use std::fmt;

#[derive(Debug)]
pub enum DingTalkError {
    /// 配置缺失（附字段名提示）。
    EmptyConfig(String),
    /// 钉钉接口返回错误（错误信息）。
    Api(String),
    /// 网络错误。
    Network(String),
    /// 响应无法解析。
    BadResponse,
}

impl fmt::Display for DingTalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DingTalkError::EmptyConfig(field) => write!(f, "{}不能为空", field),
            DingTalkError::Api(msg) => write!(f, "钉钉接口错误: {}", msg),
            DingTalkError::Network(msg) => write!(f, "网络错误: {}", msg),
            DingTalkError::BadResponse => write!(f, "无法解析钉钉响应"),
        }
    }
}

impl std::error::Error for DingTalkError {}
