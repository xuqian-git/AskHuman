#![allow(dead_code)] // 保留给高级版卡片（A 方案，后续增强）；B 方案暂不使用。
//! StandardCard 互动卡片（普通版，免搭模板）的 cardData 构造与回调解析。
//!
//! 选项做成「回传请求」按钮（点选切换 + 高亮），外加一个「发送」按钮收尾。
//! 回调走 Stream（topic `/v1.0/card/instances/callback`）。
//!
//! 说明：StandardCard 的 cardData 组件结构以钉钉实测为准，本文件提供初版结构，
//! 实现期可据真机回调微调（按钮 id / params 约定保持稳定，便于解析）。

use serde_json::{json, Value};

/// 一次卡片按钮回调的解析结果。
pub struct CardAction {
    pub user_id: String,
    pub out_track_id: String,
    /// 被点击按钮的 id（如 `opt_0` / `submit`）。
    pub action_id: String,
    /// 按钮携带的自定义参数（约定含 `kind`：`toggle` / `submit`，`value`：选项文本）。
    pub params: Value,
}

/// 选项按钮 id 前缀（便于解析时识别）。
pub const OPT_PREFIX: &str = "opt_";
pub const SUBMIT_ID: &str = "submit";

/// 构造题目卡片的 cardData（JSON 字符串）。
/// `selected` 为当前已选项（用于高亮 ✅）。
pub fn build_question_card_data(
    header: &str,
    text: &str,
    options: &[String],
    selected: &[String],
    is_markdown: bool,
) -> String {
    let mut contents: Vec<Value> = Vec::new();

    // 头部 + 正文。
    let body_md = match (header.is_empty(), text.is_empty()) {
        (true, true) => "…".to_string(),
        (false, true) => format!("**{}**", header),
        (true, false) => text.to_string(),
        (false, false) => format!("**{}**\n\n{}", header, text),
    };
    // 非 markdown 时也用 markdown 组件承载纯文本（不额外渲染语法由前置 ** 控制头部加粗）。
    let _ = is_markdown;
    contents.push(json!({ "type": "markdown", "text": body_md, "id": "q_md" }));

    // 选项按钮（每个一行 action）。
    for (i, opt) in options.iter().enumerate() {
        let checked = selected.iter().any(|s| s == opt);
        let label = if checked {
            format!("✅ {}", opt)
        } else {
            opt.clone()
        };
        let id = format!("{}{}", OPT_PREFIX, i);
        contents.push(json!({
            "type": "action",
            "id": format!("act_{}", id),
            "actions": [{
                "type": "button",
                "label": { "type": "text", "text": label },
                "id": id,
                "actionType": "callback",
                "params": { "kind": "toggle", "value": opt }
            }]
        }));
    }

    // 「发送」按钮。
    contents.push(json!({
        "type": "action",
        "id": "act_submit",
        "actions": [{
            "type": "button",
            "label": { "type": "text", "text": "发送" },
            "id": SUBMIT_ID,
            "actionType": "callback",
            "params": { "kind": "submit" }
        }]
    }));

    json!({
        "config": { "autoLayout": true, "enableForward": false },
        "contents": contents,
    })
    .to_string()
}

/// 解析卡片回调 data：取 userId / outTrackId / 按钮 id / params。
pub fn parse_card_callback(data: &Value) -> Option<CardAction> {
    let user_id = data
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let out_track_id = data
        .get("outTrackId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // value 可能是 JSON 字符串或对象。
    let value_obj: Value = match data.get("value") {
        Some(Value::String(s)) => serde_json::from_str(s).ok()?,
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let private = value_obj.get("cardPrivateData");
    let action_id = private
        .and_then(|p| p.get("actionIds"))
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let params = private
        .and_then(|p| p.get("params"))
        .cloned()
        .unwrap_or(Value::Null);

    Some(CardAction {
        user_id,
        out_track_id,
        action_id,
        params,
    })
}
