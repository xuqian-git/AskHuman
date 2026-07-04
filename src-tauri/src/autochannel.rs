//! 「IM 会话期自动激活」的与传输无关的小逻辑：活跃槽持久化、入站 slash 命令解析、
//! `/status` 文本组装、激活回执文案。
//!
//! 设计见 `docs/plans/im-channel-activation.md`。活跃槽（当前用哪个 IM 接收提问）持久化到
//! `~/.askhuman/state/auto-channel.json`，跨 daemon 重启保留，仅由「用户在某渠道的入站消息」更新。

use crate::i18n::{self, Lang};
use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 持久化的活跃槽。
#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    /// 当前活跃渠道 id（"feishu" / "dingding" / "telegram" / "slack" / "popup"）；
    /// None / "popup" = 不向任何 IM 发卡片（只弹窗）。在哪个渠道作答 / 说话就更新为哪个。
    #[serde(default)]
    channel: Option<String>,
    /// 最近一次更新时间（unix 秒，仅诊断用）。
    #[serde(default)]
    updated_at: u64,
}

/// 读取持久化的活跃槽（缺失 / 解析失败 → None）。
pub fn load_active() -> Option<String> {
    let text = std::fs::read_to_string(paths::auto_channel_file()).ok()?;
    let parsed: Persisted = serde_json::from_str(&text).ok()?;
    parsed.channel.filter(|s| !s.is_empty())
}

/// 原子写入活跃槽（临时文件 + rename）。best-effort，失败静默。
pub fn save_active(channel: Option<&str>) {
    let data = Persisted {
        channel: channel.map(|s| s.to_string()),
        updated_at: now_secs(),
    };
    let Ok(json) = serde_json::to_string_pretty(&data) else {
        return;
    };
    let path = paths::auto_channel_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4()));
    if std::fs::write(&tmp, json.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// 入站内置命令（带 `/` 前缀才算命令；可扩展）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// `/here`、`/这里`：把此渠道设为活跃槽 + 补推在途 + 必回执。
    Here,
    /// `/status`、`/状态`：`None` 返回工作中/空闲 agent 列表；`Some(编号)` 返回该 agent 的当前活动详情。
    Status(Option<u64>),
    /// `/help`、`/帮助`、`/?`：返回动态引导文案（可发什么、可用命令）。
    Help,
}

/// 一条入站文本的分类（供 `handle_inbound` 分派）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parsed {
    /// 已识别的内置命令。
    Command(Command),
    /// 以 `/` 开头但不认识的命令（armed 时不会进卡片当答案 → 安全回引导）。
    UnknownCommand,
    /// 非 `/` 开头的普通文本（可能被当作答案）。
    Text,
}

/// 解析入站文本：`trim` 后**以 `/` 开头**才进命令分派，取首个 token（大小写不敏感）匹配。
/// `/status <编号>`：第二个 token 是纯数字则解析为编号（`Some`），缺省 / 非数字则 `None`（全局列表）。
pub fn classify(text: &str) -> Parsed {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return Parsed::Text;
    }
    let mut tokens = trimmed.split_whitespace();
    let token = tokens.next().unwrap_or("");
    match token.to_ascii_lowercase().as_str() {
        "/here" | "/这里" => Parsed::Command(Command::Here),
        "/status" | "/状态" => {
            let sel = tokens.next().and_then(|s| s.parse::<u64>().ok());
            Parsed::Command(Command::Status(sel))
        }
        "/help" | "/帮助" | "/?" | "/？" => Parsed::Command(Command::Help),
        _ => Parsed::UnknownCommand,
    }
}

/// 作答内容被接受时的回执种类 / 模式（决定确认文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckKind {
    Text,
    Image,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckMode {
    /// 卡片模式：内容累积进答案，需点「提交」定稿。
    Card,
    /// 文本兜底模式：一条消息即完成该题。
    Fallback,
}

/// 「内容被接受进答案」的即时确认文案（spec R2）。仅在内容确实被接受时由渠道会话发送。
pub fn answer_ack_text(kind: AckKind, mode: AckMode, lang: Lang) -> String {
    let key = match (mode, kind) {
        (AckMode::Card, AckKind::Image) => "autoChannel.ackImageCard",
        (AckMode::Card, AckKind::File) => "autoChannel.ackFileCard",
        (AckMode::Card, AckKind::Text) => "autoChannel.ackTextCard",
        (AckMode::Fallback, AckKind::Image) => "autoChannel.ackImageFallback",
        (AckMode::Fallback, AckKind::File) => "autoChannel.ackFileFallback",
        (AckMode::Fallback, AckKind::Text) => "autoChannel.ackTextFallback",
    };
    i18n::tr(lang, key).to_string()
}

/// 自动识别 ID 成功后的回执文案（spec R5）：只报字段名、不回显 ID。
pub fn detect_ack_text(field_label: &str, lang: Lang) -> String {
    i18n::tr(lang, "autoChannel.detectAck").replace("{field}", field_label)
}

/// 动态引导 / `/help` 文案（spec R3）：按开关拼装可用命令、如何作答、切槽提示。
/// **不含「已收到」**——能回复本身即代表收到且在运行。
/// - `auto`：自动激活是否开启（决定是否列 `/here` 与切槽提示）。
/// - `has_active_question`：该渠道当前是否有在途提问（决定「如何作答」vs「暂无提问」）。
pub fn help_text(auto: bool, has_active_question: bool, lang: Lang) -> String {
    let mut out = String::new();
    out.push_str(i18n::tr(lang, "autoChannel.helpTitle"));
    out.push('\n');
    out.push_str(i18n::tr(lang, "autoChannel.helpCmdStatus"));
    out.push('\n');
    out.push_str(i18n::tr(lang, "autoChannel.helpCmdHelp"));
    if auto {
        out.push('\n');
        out.push_str(i18n::tr(lang, "autoChannel.helpCmdHere"));
    }
    out.push_str("\n\n");
    if has_active_question {
        out.push_str(i18n::tr(lang, "autoChannel.helpAnswering"));
    } else {
        out.push_str(i18n::tr(lang, "autoChannel.helpNoQuestion"));
    }
    if auto {
        out.push_str("\n\n");
        out.push_str(i18n::tr(lang, "autoChannel.helpSwitchHint"));
    }
    out
}

/// 激活回执文案：基础确认句 +（补推了 N>0 条在途时）追加补推后缀。
pub fn activated_receipt(pending: usize, lang: Lang) -> String {
    let mut s = i18n::tr(lang, "autoChannel.activated").to_string();
    if pending > 0 {
        s.push_str(&i18n::tr(lang, "autoChannel.pending").replace("{n}", &pending.to_string()));
    }
    s
}

/// 反激活提示：活跃槽切到别处时发给**旧**渠道，明确告知切到了哪个渠道（`new_id`，含 "popup"），
/// 后续提问不再走此渠道、可发 `/here` 重新激活。
pub fn deactivated_receipt(new_id: &str, lang: Lang) -> String {
    i18n::tr(lang, "autoChannel.deactivated").replace("{target}", &channel_label(new_id, lang))
}

/// 渠道 id → 展示名（复用「回复来源」文案）。未知 id 原样返回。
pub fn channel_label(id: &str, lang: Lang) -> String {
    let key = match id {
        "popup" => "channel.sourcePopup",
        "telegram" => "channel.sourceTelegram",
        "dingding" => "channel.sourceDingTalk",
        "feishu" => "channel.sourceFeishu",
        "slack" => "channel.sourceSlack",
        other => return other.to_string(),
    };
    i18n::tr(lang, key).to_string()
}

/// 由 agent 注册表快照（`AgentRegistry::snapshot()` 的 Value 数组）组装 `/status` 文本：
/// 仅列「工作中 / 空闲」（已结束不列），工作中在前；空则给「需开启生命周期追踪」提示。
pub fn status_text(snapshot: &Value, lang: Lang) -> String {
    let empty = Vec::new();
    let list = snapshot.as_array().unwrap_or(&empty);

    let mut working: Vec<String> = Vec::new();
    let mut idle: Vec<String> = Vec::new();
    for rec in list {
        let state = rec.get("state").and_then(|v| v.as_str()).unwrap_or("");
        let line = match state {
            "working" => &mut working,
            "idle" => &mut idle,
            _ => continue, // ended / 未知：不列
        };
        line.push(format_line(rec, lang));
    }

    if working.is_empty() && idle.is_empty() {
        return i18n::tr(lang, "autoChannel.statusEmpty").to_string();
    }

    let mut out = String::new();
    if !working.is_empty() {
        out.push_str(i18n::tr(lang, "autoChannel.statusWorking"));
        out.push('\n');
        out.push_str(&working.join("\n"));
    }
    if !idle.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(i18n::tr(lang, "autoChannel.statusIdle"));
        out.push('\n');
        out.push_str(&idle.join("\n"));
    }
    out
}

/// 全局列表单行：`[编号] 类型 — 标题（项目）`。
fn format_line(rec: &Value, lang: Lang) -> String {
    let seq = rec.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
    format!("[{}] {}", seq, kind_title_project(rec, lang))
}

/// `类型 — 标题（项目）`（全局行与详情头部共用）。
fn kind_title_project(rec: &Value, lang: Lang) -> String {
    let kind = rec.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let kind_label = crate::agents::AgentKind::parse(kind)
        .map(|k| k.label())
        .unwrap_or(kind);

    let title = rec
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| i18n::tr(lang, "autoChannel.noTitle").to_string());

    let project = rec
        .get("cwd")
        .and_then(|v| v.as_str())
        .and_then(project_name)
        .unwrap_or_else(|| i18n::tr(lang, "autoChannel.noProject").to_string());

    format!("{} — {}（{}）", kind_label, title, project)
}

/// `/status <编号>`：单个 agent 的「头部 + 当前活动」。找不到该编号回未找到提示。
/// 可寻址范围＝快照里的任意记录（工作中 / 空闲 / 已结束皆可）。
pub fn status_detail_text(snapshot: &Value, id: u64, lang: Lang) -> String {
    let empty = Vec::new();
    let list = snapshot.as_array().unwrap_or(&empty);
    let Some(rec) = list
        .iter()
        .find(|r| r.get("seq").and_then(|v| v.as_u64()) == Some(id))
    else {
        return i18n::tr(lang, "autoChannel.statusDetailNotFound").replace("{id}", &id.to_string());
    };

    // 头部：[编号] 类型 — 标题（项目）· 状态词
    let state = rec.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let state_word = match state {
        "working" => i18n::tr(lang, "autoChannel.stateWorking"),
        "idle" => i18n::tr(lang, "autoChannel.stateIdle"),
        "ended" => i18n::tr(lang, "autoChannel.stateEnded"),
        _ => "",
    };
    let mut out = format!("[{}] {}", id, kind_title_project(rec, lang));
    if !state_word.is_empty() {
        out.push_str(" · ");
        out.push_str(state_word);
    }

    // 当前活动：融合 transcript 尾部与 hook 实时「当前工具」。空行 + 分区标签，明确「agent 输出从这里开始」。
    let kind = rec.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let sid = rec.get("sessionId").and_then(|v| v.as_str()).unwrap_or("");
    let activity = crate::agents::AgentKind::parse(kind)
        .filter(|_| !sid.is_empty())
        .and_then(|k| crate::agents::activity::resolve_activity(k, sid));

    // transcript 侧：助手文字永远取此；工具与时间用于与实时工具比较。
    let ts_text = activity.as_ref().and_then(|a| a.text.clone());
    let ts_tool = activity.as_ref().and_then(|a| a.tool.clone());
    let ts_at = activity.as_ref().and_then(|a| a.at);

    // 实时侧：snapshot 注入的 currentTool（PreToolUse 上报、in-flight 时 transcript 尚未落盘）。
    let rt = rec.get("currentTool");
    let rt_at = rt.and_then(|t| t.get("at")).and_then(|v| v.as_u64());
    let rt_tool = rt.and_then(build_rt_tool);

    // 融合：实时工具严格更新（transcript 尚未追上）→ 用实时工具 + 其开始时间；否则用 transcript。
    let use_rt = rt_tool.is_some() && realtime_newer(rt_at, ts_at);
    let (show_tool, display_at) = if use_rt {
        (rt_tool, rt_at)
    } else {
        (ts_tool, ts_at)
    };

    out.push_str("\n\n");
    if ts_text.is_none() && show_tool.is_none() {
        out.push_str(i18n::tr(lang, "autoChannel.statusNoActivity"));
    } else {
        out.push_str(&activity_heading(display_at, lang));
        if let Some(t) = ts_text {
            out.push('\n');
            out.push_str(&t);
        }
        if let Some(tool) = show_tool {
            out.push('\n');
            out.push_str(&render_tool(&tool, lang));
        }
    }
    out
}

/// 由 snapshot 的 `currentTool`（`{name, object, at}`）构造工具展示。类别标签按原始工具名复得，
/// 对象用已归一化的存量值。无有效工具名 → None。
fn build_rt_tool(rt: &Value) -> Option<crate::agents::activity::ToolDisplay> {
    let name = rt.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
    if name.is_empty() {
        return None;
    }
    let object = rt
        .get("object")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let label = crate::agents::activity::classify_tool(name, None).label;
    Some(crate::agents::activity::ToolDisplay { label, object })
}

/// 实时工具是否比 transcript 尾部工具**更新**（严格）：`rt` 存在且时间严格晚于 `ts`，或 transcript
/// 无时间（尚未落盘）→ true。等于/更旧 → false（兜底弃用实时工具，防丢 PostToolUse 残留）。
fn realtime_newer(rt_at: Option<u64>, ts_at: Option<u64>) -> bool {
    match (rt_at, ts_at) {
        (Some(r), Some(t)) => r > t,
        (Some(_), None) => true,
        _ => false,
    }
}

/// 「最近动态」分区标签，带相对时间：zh `最近动态（3 秒前）：` / en `Latest activity (3s ago):`。
/// `at` 缺失时省略括号。
fn activity_heading(at: Option<u64>, lang: Lang) -> String {
    let heading = i18n::tr(lang, "autoChannel.activityHeading");
    let rel = at.map(|ts| rel_time(now_secs(), ts, lang));
    match (lang, rel) {
        (Lang::Zh, Some(r)) => format!("{heading}（{r}）："),
        (Lang::Zh, None) => format!("{heading}："),
        (_, Some(r)) => format!("{heading} ({r}):"),
        (_, None) => format!("{heading}:"),
    }
}

/// 相对时间标注（供「最近动态」用）。<5s → 刚刚；否则秒 / 分钟 / 小时 / 天前。
fn rel_time(now: u64, ts: u64, lang: Lang) -> String {
    let d = now.saturating_sub(ts);
    match lang {
        Lang::Zh => {
            if d < 5 {
                "刚刚".to_string()
            } else if d < 60 {
                format!("{d} 秒前")
            } else if d < 3600 {
                format!("{} 分钟前", d / 60)
            } else if d < 86400 {
                format!("{} 小时前", d / 3600)
            } else {
                format!("{} 天前", d / 86400)
            }
        }
        Lang::En => {
            if d < 5 {
                "just now".to_string()
            } else if d < 60 {
                format!("{d}s ago")
            } else if d < 3600 {
                format!("{}m ago", d / 60)
            } else if d < 86400 {
                format!("{}h ago", d / 3600)
            } else {
                format!("{}d ago", d / 86400)
            }
        }
    }
}

/// 渲染一条工具调用：`▸ <类别词/原始工具名>: <对象>`。前缀 `▸` 标示「这是一次工具调用」。
fn render_tool(tool: &crate::agents::activity::ToolDisplay, lang: Lang) -> String {
    use crate::agents::activity::ToolLabel;
    let label = match &tool.label {
        ToolLabel::Run => i18n::tr(lang, "autoChannel.activityRun").to_string(),
        ToolLabel::Read => i18n::tr(lang, "autoChannel.activityRead").to_string(),
        ToolLabel::Write => i18n::tr(lang, "autoChannel.activityWrite").to_string(),
        ToolLabel::Other(name) => name.clone(),
    };
    match &tool.object {
        Some(o) => format!("▸ {}: {}", label, o),
        None => format!("▸ {}", label),
    }
}

/// 取工作目录的末段作为项目名（空 → None）。
fn project_name(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    std::path::Path::new(trimmed)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn classify_commands_and_synonyms() {
        assert_eq!(classify("/here"), Parsed::Command(Command::Here));
        assert_eq!(classify(" /这里 "), Parsed::Command(Command::Here));
        assert_eq!(classify("/status"), Parsed::Command(Command::Status(None)));
        assert_eq!(classify("/状态"), Parsed::Command(Command::Status(None)));
        assert_eq!(classify("/help"), Parsed::Command(Command::Help));
        assert_eq!(classify("/帮助"), Parsed::Command(Command::Help));
        assert_eq!(classify("/?"), Parsed::Command(Command::Help));
        assert_eq!(classify("/？"), Parsed::Command(Command::Help));
    }

    #[test]
    fn classify_status_with_id() {
        assert_eq!(classify("/status 3"), Parsed::Command(Command::Status(Some(3))));
        assert_eq!(classify("/状态 12"), Parsed::Command(Command::Status(Some(12))));
        // 非数字参数 → 全局。
        assert_eq!(classify("/status abc"), Parsed::Command(Command::Status(None)));
    }

    #[test]
    fn classify_is_case_insensitive_and_takes_first_token() {
        assert_eq!(classify("/HELP"), Parsed::Command(Command::Help));
        // "now" 非数字 → 全局。
        assert_eq!(classify("/Status now"), Parsed::Command(Command::Status(None)));
    }

    #[test]
    fn classify_unknown_command_vs_plain_text() {
        assert_eq!(classify("/foobar"), Parsed::UnknownCommand);
        assert_eq!(classify("/"), Parsed::UnknownCommand);
        assert_eq!(classify("hello"), Parsed::Text);
        assert_eq!(classify("  not a command /here"), Parsed::Text);
        assert_eq!(classify(""), Parsed::Text);
    }

    #[test]
    fn help_text_gates_on_auto_activation() {
        let here = i18n::tr(Lang::En, "autoChannel.helpCmdHere");
        let switch = i18n::tr(Lang::En, "autoChannel.helpSwitchHint");
        // auto on → lists /here + switch hint.
        let on = help_text(true, false, Lang::En);
        assert!(on.contains(here));
        assert!(on.contains(switch));
        // auto off → neither /here nor switch hint.
        let off = help_text(false, false, Lang::En);
        assert!(!off.contains(here));
        assert!(!off.contains(switch));
    }

    #[test]
    fn help_text_gates_on_active_question() {
        let answering = i18n::tr(Lang::En, "autoChannel.helpAnswering");
        let none = i18n::tr(Lang::En, "autoChannel.helpNoQuestion");
        let with_q = help_text(false, true, Lang::En);
        assert!(with_q.contains(answering));
        assert!(!with_q.contains(none));
        let without_q = help_text(false, false, Lang::En);
        assert!(without_q.contains(none));
        assert!(!without_q.contains(answering));
    }

    #[test]
    fn answer_ack_distinguishes_kind_and_mode() {
        // Card vs Fallback differ; kinds differ.
        let img_card = answer_ack_text(AckKind::Image, AckMode::Card, Lang::En);
        let img_fb = answer_ack_text(AckKind::Image, AckMode::Fallback, Lang::En);
        assert_ne!(img_card, img_fb);
        let file_card = answer_ack_text(AckKind::File, AckMode::Card, Lang::En);
        assert_ne!(img_card, file_card);
    }

    #[test]
    fn detect_ack_inserts_field_without_id() {
        let field = i18n::tr(Lang::En, "autoChannel.detectFieldUserId");
        let out = detect_ack_text(field, Lang::En);
        assert!(out.contains(field));
        assert!(!out.contains("{field}"));
    }

    #[test]
    fn status_text_prefixes_seq() {
        let snap = serde_json::json!([
            {"seq":2,"kind":"cursor","sessionId":"s","state":"working","title":"t","cwd":"/a/proj"}
        ]);
        let out = status_text(&snap, Lang::En);
        assert!(out.contains("[2] "));
    }

    #[test]
    fn rel_time_buckets_and_heading() {
        let base = 1_000_000u64;
        assert_eq!(rel_time(base + 2, base, Lang::Zh), "刚刚");
        assert_eq!(rel_time(base + 30, base, Lang::Zh), "30 秒前");
        assert_eq!(rel_time(base + 120, base, Lang::Zh), "2 分钟前");
        assert_eq!(rel_time(base + 7200, base, Lang::Zh), "2 小时前");
        assert_eq!(rel_time(base + 30, base, Lang::En), "30s ago");
        // 标签带相对时间括号；缺时间省略括号。
        let h = activity_heading(Some(now_secs()), Lang::Zh);
        assert!(h.starts_with("最近动态（"));
        assert!(h.ends_with("）："));
        assert_eq!(activity_heading(None, Lang::Zh), "最近动态：");
        assert_eq!(activity_heading(None, Lang::En), "Latest activity:");
    }

    #[test]
    fn realtime_tool_fusion_decision() {
        // 实时更新（严格）→ 用实时。
        assert!(realtime_newer(Some(100), Some(90)));
        // transcript 无时间（尚未落盘）→ 用实时。
        assert!(realtime_newer(Some(100), None));
        // 相等 / 更旧 → 弃用实时（用 transcript）。
        assert!(!realtime_newer(Some(100), Some(100)));
        assert!(!realtime_newer(Some(80), Some(100)));
        assert!(!realtime_newer(None, Some(100)));
        // build_rt_tool：有名出工具（Shell→运行命令类别），无名 None。
        let td = build_rt_tool(&serde_json::json!({"name":"Shell","object":"cargo test","at":1}))
            .unwrap();
        assert_eq!(td.label, crate::agents::activity::ToolLabel::Run);
        assert_eq!(td.object.as_deref(), Some("cargo test"));
        assert!(build_rt_tool(&serde_json::json!({"name":"","at":1})).is_none());
    }

    #[test]
    fn status_detail_not_found_and_no_activity() {
        let snap = serde_json::json!([
            {"seq":1,"kind":"cursor","sessionId":"no-such-session-xyz","state":"working","title":"做点事","cwd":"/tmp/proj"}
        ]);
        // 未找到编号 → 含 id 提示。
        let nf = status_detail_text(&snap, 9, Lang::En);
        assert!(nf.contains('9'));
        // 命中：头部含编号与标题；无会话文件 → 无活动提示。
        let d = status_detail_text(&snap, 1, Lang::En);
        assert!(d.contains("[1]"));
        assert!(d.contains("做点事"));
        assert!(d.contains(i18n::tr(Lang::En, "autoChannel.statusNoActivity")));
    }
}
