//! Telegram Channel：发送提问 + 长轮询接收回复（不接收图片），逐项对齐 Swift 版。

use super::{Channel, ResultSink};
use crate::config::TelegramChannelConfig;
use crate::models::{AskRequest, ChannelAction, ChannelResult, MessagePrompt, QuestionAnswer};
use crate::telegram::{markdown, TelegramClient};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const SEND_BUTTON: &str = "↗️发送";

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
            run_session(request, config, cancelled, sink).await;
        });
    }

    fn cancel_by_other(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

pub(crate) async fn run_session(
    request: AskRequest,
    config: TelegramChannelConfig,
    cancelled: Arc<AtomicBool>,
    sink: ResultSink,
) {
    let client = match TelegramClient::new(config.bot_token, config.chat_id, config.api_base_url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("警告: Telegram 配置无效，已跳过该 Channel: {}", e);
            return;
        }
    };

    let n = request.questions.len();
    let has_message =
        !request.message.text.trim().is_empty() || !request.message.files.is_empty();
    let source = crate::models::source_name();

    let mut answers: Vec<QuestionAnswer> = Vec::with_capacity(n);
    // 长轮询 offset 跨问题持续递增。
    let mut offset: i64 = 0;

    if n == 1 && !has_message {
        // 单题且无 Message：现状单条，问题自带「Question from {名}」来源头部。
        let q = &request.questions[0];
        let header = format!("「Question from {}」", source);
        match ask_question(
            &client,
            &header,
            &q.message,
            &q.predefined_options,
            request.is_markdown,
            &cancelled,
            &mut offset,
        )
        .await
        {
            Some(answer) => answers.push(answer),
            None => return,
        }
    } else {
        // 先发共享 Message（头部 + 文本 + 文件），再逐题串行。
        send_message_prompt(&client, &request.message, request.is_markdown, &source).await;
        for (index, question) in request.questions.iter().enumerate() {
            let header = if n > 1 {
                format!("Question {}/{}", index + 1, n)
            } else {
                String::new()
            };
            match ask_question(
                &client,
                &header,
                &question.message,
                &question.predefined_options,
                request.is_markdown,
                &cancelled,
                &mut offset,
            )
            .await
            {
                Some(answer) => answers.push(answer),
                // 被其它 channel 抢答（cancelled）→ 中止，不投递。
                None => return,
            }
        }
    }

    sink.submit(ChannelResult {
        action: ChannelAction::Send,
        answers,
        source_channel_id: "telegram".to_string(),
    });
}

/// 发送共享 Message：头部「Question from {名}」+（文本，若有）+ 其展示文件。
async fn send_message_prompt(
    client: &TelegramClient,
    message: &MessagePrompt,
    is_markdown: bool,
    source: &str,
) {
    let header = format!("「Question from {}」", source);
    send_composed(client, &header, &message.text, is_markdown, None).await;

    // 发送 Message 的展示文件（图片→sendPhoto，其它→sendDocument）。
    for file in &message.files {
        let result = if file.is_image {
            client.send_photo(&file.path, &file.name).await
        } else {
            client.send_document(&file.path, &file.name).await
        };
        if let Err(e) = result {
            eprintln!("警告: 文件发送失败: {}: {}", file.path, e);
            let _ = client
                .send_message(&format!("⚠️ 文件发送失败：{}", file.path), None, None)
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
    cancelled: &Arc<AtomicBool>,
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
            "在键盘上点「发送」完成回复，或直接回复文字补充说明",
            None,
            Some(reply_keyboard()),
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
            if text == SEND_BUTTON {
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

fn reply_keyboard() -> Value {
    json!({
        "keyboard": [[{ "text": SEND_BUTTON }]],
        "resize_keyboard": true,
        "one_time_keyboard": true
    })
}
