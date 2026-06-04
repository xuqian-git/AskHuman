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

/// 一次提问请求：一个共享 Message（描述 + 附件）+ 一组问题（恒 ≥1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    pub id: String,
    /// 是否按 Markdown 渲染（全局，对所有问题生效）。
    pub is_markdown: bool,
    /// 共享 Message：所有问题的描述与展示附件。
    pub message: MessagePrompt,
    /// 问题列表（恒 ≥1，由 CLI 归一化保证：无 `-q` 时由第一个参数提升而来）。
    #[serde(default)]
    pub questions: Vec<Question>,
}

impl AskRequest {
    pub fn new(message: MessagePrompt, questions: Vec<Question>, is_markdown: bool) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            is_markdown,
            message,
            questions,
        }
    }
}

/// 共享 Message：所有问题的描述文本与展示附件（不持有选项）。
///
/// `text` 仅在使用了 `-q`（存在独立描述）时非空；无 `-q` 时第一个参数被提升为问题，`text` 为空。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePrompt {
    #[serde(default)]
    pub text: String,
    /// 提问附带的文件（AI→人，仅用于弹窗展示，不进入结果输出）。
    #[serde(default)]
    pub files: Vec<FileAttachment>,
}

impl MessagePrompt {
    pub fn new(text: String, files: Vec<FileAttachment>) -> Self {
        Self { text, files }
    }
}

/// 单个问题（其选项与该问题绑定；是否 Markdown 见 `AskRequest::is_markdown`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub message: String,
    #[serde(default)]
    pub predefined_options: Vec<String>,
}

impl Question {
    pub fn new(message: String, predefined_options: Vec<String>) -> Self {
        Self {
            message,
            predefined_options,
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

/// 对单个问题的回答。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    #[serde(default)]
    pub selected_options: Vec<String>,
    #[serde(default)]
    pub user_input: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageAttachment>,
    /// 用户随回复附带的本地文件绝对路径（非图片，直接透传不复制）。
    #[serde(default)]
    pub files: Vec<String>,
}

impl QuestionAnswer {
    /// 是否为空回答：没选项、没（去空白后的）输入、没图片、没回复文件。
    pub fn is_empty(&self) -> bool {
        self.selected_options.is_empty()
            && self
                .user_input
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            && self.images.is_empty()
            && self.files.is_empty()
    }
}

/// 某个 Channel 给出的最终回答（按问题顺序，每题一项）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelResult {
    pub action: ChannelAction,
    #[serde(default)]
    pub answers: Vec<QuestionAnswer>,
    pub source_channel_id: String,
}

impl ChannelResult {
    pub fn cancel(source_channel_id: impl Into<String>) -> Self {
        Self {
            action: ChannelAction::Cancel,
            answers: Vec::new(),
            source_channel_id: source_channel_id.into(),
        }
    }
}
