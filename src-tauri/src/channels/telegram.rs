//! Telegram Channel：发送提问 + 长轮询接收回复（不接收图片），逐项对齐 Swift 版。
//!
//! 编排逻辑（单/多题、收集答案、投递）已上移到 `channels::conversation::run_conversation`；
//! 本文件提供传输相关实现 `TelegramSession`（`MessagingChannel`）+ 薄外层 `TelegramChannel`。

use super::conversation::{run_conversation, MessagingChannel, QuestionCtx};
use super::{Channel, ResultSink};
use crate::config::TelegramChannelConfig;
use crate::i18n::{self, Lang};
use crate::models::{AskRequest, MessagePrompt, QuestionAnswer};
use crate::telegram::{markdown, TelegramClient};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 「发送」键盘按钮文案（按界面语言）。发送与回复比对必须用同一语言取值。
fn send_button(lang: Lang) -> String {
    i18n::tr(lang, "channel.tgSendButton").to_string()
}

/// 薄外层：接 Coordinator（并行抢答），把会话委托给 `run_conversation` + `TelegramSession`。
pub struct TelegramChannel {
    config: TelegramChannelConfig,
    cancelled: Arc<AtomicBool>,
}

impl TelegramChannel {
    pub fn new(config: TelegramChannelConfig) -> Self {
        Self {
            config,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Channel for TelegramChannel {
    fn id(&self) -> &str {
        "telegram"
    }

    fn start(&self, request: &AskRequest, sink: ResultSink) {
        let config = self.config.clone();
        let cancelled = self.cancelled.clone();
        let request = request.clone();
        tauri::async_runtime::spawn(async move {
            let mut session = TelegramSession::new(config);
            if let Err(e) = session.open().await {
                let lang = Lang::current();
                eprintln!(
                    "{}{}",
                    i18n::warn_prefix(lang),
                    i18n::tr(lang, "channel.tgConfigInvalidSkip").replace("{e}", &e.to_string())
                );
                return;
            }
            run_conversation(&mut session, &request, cancelled, sink).await;
        });
    }

    fn cancel_by_other(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// 传输实现：持有 client 与跨题长轮询 offset。
pub struct TelegramSession {
    config: TelegramChannelConfig,
    client: Option<TelegramClient>,
    offset: i64,
}

impl TelegramSession {
    pub fn new(config: TelegramChannelConfig) -> Self {
        Self {
            config,
            client: None,
            offset: 0,
        }
    }
}

#[async_trait::async_trait]
impl MessagingChannel for TelegramSession {
    fn id(&self) -> &str {
        "telegram"
    }

    async fn open(&mut self) -> Result<(), String> {
        let client = TelegramClient::new(
            self.config.bot_token.clone(),
            self.config.chat_id.clone(),
            self.config.api_base_url.clone(),
        )
        .map_err(|e| e.to_string())?;
        self.client = Some(client);
        Ok(())
    }

    async fn send_message_prompt(
        &mut self,
        message: &MessagePrompt,
        is_markdown: bool,
        source: &str,
        lang: Lang,
    ) {
        if let Some(client) = self.client.as_ref() {
            send_message_prompt(client, message, is_markdown, source, lang).await;
        }
    }

    async fn ask_question(
        &mut self,
        ctx: &QuestionCtx<'_>,
        cancelled: &AtomicBool,
    ) -> Option<QuestionAnswer> {
        // 拆分借用：client 不可变 + offset 可变。
        let Self { client, offset, .. } = self;
        let client = client.as_ref()?;
        ask_question(
            client,
            ctx.header,
            ctx.text,
            ctx.options,
            ctx.is_markdown,
            ctx.lang,
            cancelled,
            offset,
        )
        .await
    }

    async fn close(&mut self) {}
}

/// 发送共享 Message：头部「Question from {名}」+（文本，若有）+ 其展示文件。
async fn send_message_prompt(
    client: &TelegramClient,
    message: &MessagePrompt,
    is_markdown: bool,
    source: &str,
    lang: Lang,
) {
    let header = format!(
        "「{}」",
        i18n::tr(lang, "channel.messageFrom").replace("{source}", source)
    );
    send_composed(client, &header, &message.text, is_markdown, None).await;

    // 发送 Message 的展示文件（图片→sendPhoto，其它→sendDocument）。
    for file in &message.files {
        let result = if file.is_image {
            client.send_photo(&file.path, &file.name).await
        } else {
            client.send_document(&file.path, &file.name).await
        };
        if let Err(e) = result {
            eprintln!(
                "{}{}",
                i18n::warn_prefix(lang),
                i18n::tr(lang, "channel.fileSendFailedLog")
                    .replace("{path}", &file.path)
                    .replace("{e}", &e.to_string())
            );
            let _ = client
                .send_message(
                    &i18n::tr(lang, "channel.fileSendFailed").replace("{name}", &file.path),
                    None,
                    None,
                )
                .await;
        }
    }
}

/// 发送一道题（选项消息 + 操作消息）并长轮询直到用户点「发送」；返回该题回答。
/// `header` 为题首加粗行（来源头部或 `Question i/n`），为空则只发问题正文。
/// 被抢答（cancelled）时返回 None。
async fn ask_question(
    client: &TelegramClient,
    header: &str,
    question_text: &str,
    options: &[String],
    is_markdown: bool,
    lang: Lang,
    cancelled: &AtomicBool,
    offset: &mut i64,
) -> Option<QuestionAnswer> {
    let options = options.to_vec();
    let mut selected: Vec<String> = Vec::new();
    let mut user_input = String::new();

    // 1. 选项消息（MarkdownV2 失败回退纯文本）。
    let inline = if options.is_empty() {
        None
    } else {
        Some(inline_keyboard(&options, &selected))
    };
    let options_message_id =
        send_composed(client, header, question_text, is_markdown, inline).await;

    // 2. 操作消息（含「发送」按钮）
    let operation_message_id = client
        .send_message(
            i18n::tr(lang, "channel.tgActionHint"),
            None,
            Some(reply_keyboard(lang)),
        )
        .await
        .unwrap_or(0);

    // 3. 长轮询
    while !cancelled.load(Ordering::SeqCst) {
        match client.get_updates(*offset).await {
            Ok(updates) => {
                for update in updates {
                    if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
                        *offset = uid + 1;
                    }
                    if handle_update(
                        &update,
                        client,
                        &options,
                        &mut selected,
                        &mut user_input,
                        options_message_id,
                        operation_message_id,
                        lang,
                    )
                    .await
                    {
                        return Some(QuestionAnswer {
                            selected_options: selected,
                            user_input: if user_input.is_empty() {
                                None
                            } else {
                                Some(user_input)
                            },
                            images: Vec::new(),
                            files: Vec::new(),
                        });
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    None
}

/// 组装「加粗头部 + 正文」并发送（MarkdownV2 失败回退纯文本），返回消息 id。
/// `header`/`body` 任一为空时自动省略对应部分；都为空时用占位符避免空消息。
async fn send_composed(
    client: &TelegramClient,
    header: &str,
    body: &str,
    is_markdown: bool,
    inline: Option<Value>,
) -> i64 {
    let plain = match (header.is_empty(), body.is_empty()) {
        (true, true) => "…".to_string(),
        (false, true) => header.to_string(),
        (true, false) => body.to_string(),
        (false, false) => format!("{}\n\n{}", header, body),
    };
    // markdown 正文交给 markdown::process；非 markdown 正文整体转义；头部始终加粗。
    let md = if is_markdown {
        match (header.is_empty(), body.is_empty()) {
            (true, true) => "…".to_string(),
            (false, true) => markdown::process(&format!("**{}**", header)),
            (true, false) => markdown::process(body),
            (false, false) => markdown::process(&format!("**{}**\n\n{}", header, body)),
        }
    } else {
        match (header.is_empty(), body.is_empty()) {
            (true, true) => "…".to_string(),
            (false, true) => format!("*{}*", markdown::escape_all(header)),
            (true, false) => markdown::escape_all(body),
            (false, false) => format!(
                "*{}*\n\n{}",
                markdown::escape_all(header),
                markdown::escape_all(body)
            ),
        }
    };
    match client
        .send_message(&md, Some("MarkdownV2"), inline.clone())
        .await
    {
        Ok(id) => id,
        Err(_) => client.send_message(&plain, None, inline).await.unwrap_or(0),
    }
}

/// 处理一条 update；返回 true 表示已终结（用户点「发送」）。
async fn handle_update(
    update: &Value,
    client: &TelegramClient,
    options: &[String],
    selected: &mut Vec<String>,
    user_input: &mut String,
    options_message_id: i64,
    operation_message_id: i64,
    lang: Lang,
) -> bool {
    // callback_query：切换选项
    if let Some(cb) = update.get("callback_query") {
        if let Some(chat_id) = cb
            .get("message")
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())
        {
            if chat_id != client.chat_id() {
                return false;
            }
        }
        if let Some(data) = cb.get("data").and_then(|d| d.as_str()) {
            if let Some(opt) = data.strip_prefix("toggle:") {
                toggle(selected, opt);
                client
                    .edit_message_reply_markup(
                        options_message_id,
                        inline_keyboard(options, selected.as_slice()),
                    )
                    .await;
            }
        }
        if let Some(cb_id) = cb.get("id").and_then(|i| i.as_str()) {
            client.answer_callback_query(cb_id).await;
        }
        return false;
    }

    // message：文本回复 / 发送
    if let Some(message) = update.get("message") {
        let chat_ok = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_i64())
            == Some(client.chat_id());
        if !chat_ok {
            return false;
        }
        if let Some(msg_id) = message.get("message_id").and_then(|v| v.as_i64()) {
            if msg_id <= operation_message_id {
                return false;
            }
        }
        if let Some(text) = message.get("text").and_then(|t| t.as_str()) {
            if text == send_button(lang) {
                return true;
            }
            *user_input = text.to_string();
        }
    }
    false
}

fn toggle(selected: &mut Vec<String>, option: &str) {
    if let Some(i) = selected.iter().position(|s| s == option) {
        selected.remove(i);
    } else {
        selected.push(option.to_string());
    }
}

fn inline_keyboard(options: &[String], selected: &[String]) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < options.len() {
        let mut row: Vec<Value> = Vec::new();
        for option in &options[i..(i + 2).min(options.len())] {
            let text = if selected.iter().any(|s| s == option) {
                format!("✅ {}", option)
            } else {
                option.clone()
            };
            row.push(json!({ "text": text, "callback_data": format!("toggle:{}", option) }));
        }
        rows.push(Value::Array(row));
        i += 2;
    }
    json!({ "inline_keyboard": rows })
}

fn reply_keyboard(lang: Lang) -> Value {
    json!({
        "keyboard": [[{ "text": send_button(lang) }]],
        "resize_keyboard": true,
        "one_time_keyboard": true
    })
}
