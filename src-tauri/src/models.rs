//! 核心数据模型（与前端 TS 类型一一对应，serde 采用 camelCase）。

use serde::{Deserialize, Serialize};

/// 标题来源名的默认值（环境变量未设置或为空时使用）。
pub const DEFAULT_SOURCE_NAME: &str = "the Loop";
/// 定制标题来源名的环境变量：影响弹窗标题与 Telegram 消息头。
pub const SOURCE_NAME_ENV: &str = "ASKHUMAN_ENV_SOURCE_NAME";

/// 读取来源名：「Question from {source_name}」。环境变量为空或缺省时回退默认值。
pub fn source_name() -> String {
    std::env::var(SOURCE_NAME_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SOURCE_NAME.to_string())
}

/// 一次提问请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    pub id: String,
    pub message: String,
    pub predefined_options: Vec<String>,
    pub is_markdown: bool,
    /// 提问附带的文件（AI→人，仅用于弹窗展示，不进入结果输出）。
    #[serde(default)]
    pub files: Vec<FileAttachment>,
}

impl AskRequest {
    pub fn new(
        message: String,
        predefined_options: Vec<String>,
        is_markdown: bool,
        files: Vec<FileAttachment>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            message,
            predefined_options,
            is_markdown,
            files,
        }
    }
}

/// 提问附带的文件附件（展示用）。`path` 为绝对路径。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAttachment {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_image: bool,
}

/// 图片附件。`data` 为 base64，可带 `data:...;base64,` 前缀（落盘时清理）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
    #[serde(default)]
    pub filename: Option<String>,
}

/// Channel 的终态动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelAction {
    Send,
    Cancel,
}

/// 某个 Channel 给出的最终回答。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelResult {
    pub action: ChannelAction,
    #[serde(default)]
    pub selected_options: Vec<String>,
    #[serde(default)]
    pub user_input: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageAttachment>,
    /// 用户随回复附带的本地文件绝对路径（非图片，直接透传不复制）。
    #[serde(default)]
    pub files: Vec<String>,
    pub source_channel_id: String,
}

impl ChannelResult {
    pub fn cancel(source_channel_id: impl Into<String>) -> Self {
        Self {
            action: ChannelAction::Cancel,
            selected_options: Vec::new(),
            user_input: None,
            images: Vec::new(),
            files: Vec::new(),
            source_channel_id: source_channel_id.into(),
        }
    }
}
