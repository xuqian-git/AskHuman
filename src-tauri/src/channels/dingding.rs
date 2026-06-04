//! 钉钉 Channel：Stream 长连接收（用户文字/图片/文件）+ OpenAPI 发（Message 文本/文件、逐题提问）。
//!
//! 方案 B（当前）：提问以「纯文本 + 编号选项」下发，用户**回复一条消息即完成该题**——
//! 回复编号（多选用逗号）映射预定义选项，或直接输入文字，或发送图片/文件。
//! 钉钉互动卡片「普通版」不支持 Stream 回调；按钮快捷回复（高级版卡片 A 方案）作为后续增强，
//! 相关构造/发送/回调解析代码暂以 `#[allow(dead_code)]` 保留（见 `dingtalk::card` / `client::send_card`）。
//!
//! 编排逻辑复用 `conversation::run_conversation`，本文件提供传输实现 `DingTalkSession`
//! （`MessagingChannel`）+ 薄外层 `DingTalkChannel`。

use super::conversation::{run_conversation, MessagingChannel, QuestionCtx};
use super::{Channel, ResultSink};
use crate::config::DingTalkChannelConfig;
use crate::dingtalk::client::DingTalkClient;
use crate::dingtalk::stream::{StreamConn, StreamEvent, TOPIC_BOT_MESSAGE};
use crate::models::{ImageAttachment, MessagePrompt, QuestionAnswer};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 抢答轮询粒度：每隔此时长检查一次 `cancelled`。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 薄外层：接 Coordinator（并行抢答），把会话委托给 `run_conversation` + `DingTalkSession`。
pub struct DingTalkChannel {
    config: DingTalkChannelConfig,
    cancelled: Arc<AtomicBool>,
}

impl DingTalkChannel {
    pub fn new(config: DingTalkChannelConfig) -> Self {
        Self {
            config,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Channel for DingTalkChannel {
    fn id(&self) -> &str {
        "dingding"
    }

    fn start(&self, request: &crate::models::AskRequest, sink: ResultSink) {
        let config = self.config.clone();
        let cancelled = self.cancelled.clone();
        let request = request.clone();
        tauri::async_runtime::spawn(async move {
            let mut session = DingTalkSession::new(config);
            if let Err(e) = session.open().await {
                eprintln!("警告: 钉钉配置无效，已跳过该 Channel: {}", e);
                return;
            }
            run_conversation(&mut session, &request, cancelled, sink).await;
        });
    }

    fn cancel_by_other(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// 传输实现：持有 client 与 Stream 长连接（跨题复用）。
pub struct DingTalkSession {
    config: DingTalkChannelConfig,
    client: Option<DingTalkClient>,
    stream: Option<StreamConn>,
}

impl DingTalkSession {
    pub fn new(config: DingTalkChannelConfig) -> Self {
        Self {
            config,
            client: None,
            stream: None,
        }
    }
}

#[async_trait::async_trait]
impl MessagingChannel for DingTalkSession {
    fn id(&self) -> &str {
        "dingding"
    }

    async fn open(&mut self) -> Result<(), String> {
        let client = DingTalkClient::new(&self.config).map_err(|e| e.to_string())?;
        // B 方案只需接收用户消息（文字/图片/文件）；卡片回调 topic 暂不订阅。
        let stream = StreamConn::connect(
            client.http().clone(),
            self.config.client_id.trim(),
            self.config.client_secret.trim(),
            &[TOPIC_BOT_MESSAGE],
        )
        .await
        .map_err(|e| e.to_string())?;
        self.client = Some(client);
        self.stream = Some(stream);
        Ok(())
    }

    async fn send_message_prompt(
        &mut self,
        message: &MessagePrompt,
        is_markdown: bool,
        source: &str,
    ) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let header = format!("Question from {}", source);
        let result = if is_markdown {
            let body = if message.text.trim().is_empty() {
                format!("**{}**", header)
            } else {
                format!("**{}**\n\n{}", header, message.text)
            };
            client.send_oto_markdown(&header, &body).await
        } else {
            let body = if message.text.trim().is_empty() {
                header.clone()
            } else {
                format!("{}\n\n{}", header, message.text)
            };
            client.send_oto_text(&body).await
        };
        if let Err(e) = result {
            eprintln!("警告: 钉钉 Message 发送失败: {}", e);
        }

        // 展示文件：上传媒体后图片→sampleImageMsg，其它→sampleFile。
        for file in &message.files {
            if let Err(e) = send_attachment(client, &file.path, &file.name, file.is_image).await {
                eprintln!("警告: 钉钉文件发送失败: {}: {}", file.path, e);
                let _ = client
                    .send_oto_text(&format!("⚠️ 文件发送失败：{}", file.name))
                    .await;
            }
        }
    }

    async fn ask_question(
        &mut self,
        ctx: &QuestionCtx<'_>,
        cancelled: &AtomicBool,
    ) -> Option<QuestionAnswer> {
        let Self {
            client,
            stream,
            config,
        } = self;
        let client = client.as_ref()?;
        let stream = stream.as_mut()?;
        let user_id = config.user_id.trim().to_string();

        // 1. 发题：头部 + 正文 + 编号选项 + 作答提示。
        let title = if ctx.header.is_empty() { "提问" } else { ctx.header };
        let body = build_question_text(ctx, ctx.is_markdown);
        let send_res = if ctx.is_markdown {
            client.send_oto_markdown(title, &body).await
        } else {
            client.send_oto_text(&body).await
        };
        if let Err(e) = send_res {
            eprintln!("警告: 钉钉提问发送失败: {}", e);
        }

        // 2. 等用户「一条消息」作答（编号/文字/图片/文件）；被抢答则返回 None。
        while !cancelled.load(Ordering::SeqCst) {
            let ev = match tokio::time::timeout(POLL_INTERVAL, stream.recv()).await {
                Ok(Some(ev)) => ev,
                Ok(None) => break,    // 连接彻底断开
                Err(_) => continue,   // 超时：回到循环顶部重新检查 cancelled
            };
            if let StreamEvent::BotMessage(data) = ev {
                if !bot_message_belongs(&data, &user_id) {
                    continue;
                }
                if let Some(answer) = message_to_answer(client, &data, ctx.options).await {
                    return Some(answer);
                }
                // 非可作答消息（贴纸等）→ 继续等待。
            }
        }
        None
    }

    async fn close(&mut self) {
        self.stream = None;
    }
}

/// 组装提问正文：头部（加粗）+ 正文 + 编号选项 + 作答提示。
fn build_question_text(ctx: &QuestionCtx<'_>, is_markdown: bool) -> String {
    let mut s = String::new();
    if !ctx.header.is_empty() {
        if is_markdown {
            s.push_str(&format!("**{}**", ctx.header));
        } else {
            s.push_str(ctx.header);
        }
        s.push_str("\n\n");
    }
    if !ctx.text.is_empty() {
        s.push_str(ctx.text);
        s.push_str("\n\n");
    }
    if ctx.options.is_empty() {
        s.push_str("👉 直接回复文字即可；也可发送图片 / 文件");
    } else {
        for (i, opt) in ctx.options.iter().enumerate() {
            s.push_str(&format!("{}. {}\n", i + 1, opt));
        }
        s.push('\n');
        s.push_str("👉 回复编号选择（多选用逗号，如 1,3），或直接输入文字；也可发送图片 / 文件");
    }
    s.trim_end().to_string()
}

/// 把一条 bot 消息转成回答；非可作答类型返回 None（继续等待）。
async fn message_to_answer(
    client: &DingTalkClient,
    data: &Value,
    options: &[String],
) -> Option<QuestionAnswer> {
    let msgtype = data.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
    match msgtype {
        "text" => {
            let content = data
                .get("text")
                .and_then(|t| t.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .trim();
            if content.is_empty() {
                return None;
            }
            let (selected, user_input) = parse_reply(content, options);
            Some(QuestionAnswer {
                selected_options: selected,
                user_input,
                images: Vec::new(),
                files: Vec::new(),
            })
        }
        "picture" => {
            let code = data
                .get("content")
                .and_then(|c| c.get("downloadCode"))
                .and_then(|v| v.as_str())?;
            match download_image(client, code).await {
                Ok(img) => Some(QuestionAnswer {
                    selected_options: Vec::new(),
                    user_input: None,
                    images: vec![img],
                    files: Vec::new(),
                }),
                Err(e) => {
                    eprintln!("警告: 钉钉图片下载失败: {}", e);
                    None
                }
            }
        }
        "file" => {
            let content = data.get("content");
            let code = content
                .and_then(|c| c.get("downloadCode"))
                .and_then(|v| v.as_str())?;
            let file_name = content
                .and_then(|c| c.get("fileName"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            let ext = ext_of(file_name);
            match client.download_message_file_to(code, ext).await {
                Ok(path) => Some(QuestionAnswer {
                    selected_options: Vec::new(),
                    user_input: None,
                    images: Vec::new(),
                    files: vec![path],
                }),
                Err(e) => {
                    eprintln!("警告: 钉钉文件下载失败: {}", e);
                    None
                }
            }
        }
        _ => None,
    }
}

/// 解析文字回复：纯编号（含逗号/空格分隔）→ 映射选项；否则整条作为自由文本。
fn parse_reply(text: &str, options: &[String]) -> (Vec<String>, Option<String>) {
    let trimmed = text.trim();
    let selection_only = !options.is_empty()
        && trimmed.chars().all(|c| {
            c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(c, ',' | '，' | '、' | '/' | ';' | '；' | '.' | '。')
        });
    if selection_only {
        let mut selected: Vec<String> = Vec::new();
        for tok in trimmed.split(|c: char| !c.is_ascii_digit()) {
            if tok.is_empty() {
                continue;
            }
            if let Ok(n) = tok.parse::<usize>() {
                if n >= 1 && n <= options.len() {
                    let opt = options[n - 1].clone();
                    if !selected.contains(&opt) {
                        selected.push(opt);
                    }
                }
            }
        }
        if !selected.is_empty() {
            return (selected, None);
        }
    }
    (Vec::new(), Some(trimmed.to_string()))
}

/// 判定 bot 消息是否来自目标用户。
fn bot_message_belongs(data: &Value, user_id: &str) -> bool {
    data.get("senderStaffId").and_then(|v| v.as_str()).unwrap_or("") == user_id
}

/// 下载图片消息并转为 `ImageAttachment`（raw base64 + media_type）。
async fn download_image(client: &DingTalkClient, code: &str) -> Result<ImageAttachment, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let path = client
        .download_message_file_to(code, "png")
        .await
        .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(ImageAttachment {
        data: B64.encode(bytes),
        media_type: "image/png".to_string(),
        filename: None,
    })
}

/// 上传并发送一个附件（图片/文件）。
async fn send_attachment(
    client: &DingTalkClient,
    path: &str,
    name: &str,
    is_image: bool,
) -> Result<(), crate::dingtalk::DingTalkError> {
    if is_image {
        let media_id = client.upload_media(path, "image").await?;
        client.send_oto_image(&media_id).await
    } else {
        let media_id = client.upload_media(path, "file").await?;
        let ext = ext_of(name);
        client.send_oto_file(&media_id, name, ext).await
    }
}

/// 取文件扩展名（无则空串）。
fn ext_of(name: &str) -> &str {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
}
