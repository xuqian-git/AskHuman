//! 解析各家 agent 会话 transcript 的**尾部**，得到「当前在做什么」：最后一段助手文字 +（若末尾是
//! 工具调用则附）该次工具调用。供 IM `/status <编号>` 展示（设计见 `docs/plans/im-status-activity.md`）。
//!
//! 规则：只要助手在会话里输出过文字，就**永远**带上「最后一段助手文字」；若 transcript 末尾是工具调用
//! （含「工具刚跑完、助手尚未回话」）则再附该次工具调用；若末尾是文字则只给文字。
//!
//! 全部 best-effort：文件不存在 / 正在写 / 巨大 / 解析失败都尽量降级；尾部读取有字节上限，不拖慢 daemon。

use serde_json::Value;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::title::transcript_path;
use super::AgentKind;

/// 尾部读取的字节上限（只关心最新事件，对超大 transcript 有界）。
const MAX_TAIL_BYTES: u64 = 256 * 1024;
/// 「最后一段助手文字」最大展示字符数。
const MAX_ACTIVITY_TEXT_CHARS: usize = 500;
/// 工具对象（文件名 / 命令首段 / 参数前段）最大展示字符数。
const MAX_TOOL_OBJECT_CHARS: usize = 60;

/// 一次「当前活动」解析结果。`text`/`tool` 至少一个为 `Some` 时才会返回（否则 `resolve_activity` 返回 `None`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    /// 最后一段助手自然语言文字（已截断）。
    pub text: Option<String>,
    /// 末尾工具调用（仅当 transcript 末尾是工具调用 / 工具结果时）。
    pub tool: Option<ToolDisplay>,
    /// 最后活动时间（transcript 文件 mtime 的 Unix 秒；取不到为 `None`）。
    /// 用文件写入时间作「最近一次事件」的通用代理：各家 transcript 每次事件都会追加写盘，
    /// 无需逐家解析事件时间戳；对 Cursor「工具跑完才落盘」也与展示内容（最后完成事件）一致。
    pub at: Option<u64>,
}

/// 一次工具调用的展示信息。类别词与前缀符号的本地化渲染在 `autochannel`（`object` 是内容，不本地化）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDisplay {
    pub label: ToolLabel,
    /// 简短对象：文件名 / 命令首段 / 参数前段（已截断）。
    pub object: Option<String>,
}

/// 工具类别。只归一化常见工具；其余保留原始工具名。
#[derive(Debug, Clone, PartialEq)]
pub enum ToolLabel {
    /// 运行命令（Bash/Shell/…）。
    Run,
    /// 读取文件。
    Read,
    /// 写入 / 编辑文件。
    Write,
    /// 其它工具：携带原始工具名。
    Other(String),
}

/// 解析某家 agent 某 session 的「当前活动」。取不到（文件缺失 / 无文字也无工具）返回 `None`。
pub fn resolve_activity(kind: AgentKind, session_id: &str) -> Option<Activity> {
    let path = transcript_path(kind, session_id)?;
    let lines = read_tail(&path, MAX_TAIL_BYTES);
    let mut activity = analyze(kind, &lines)?;
    activity.at = file_mtime_secs(&path);
    Some(activity)
}

/// 取文件 mtime 的 Unix 秒（best-effort）。
fn file_mtime_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// 读文件尾部至多 `max_bytes` 字节，按行返回；若从中间切入则丢弃首个可能不完整的行。
fn read_tail(path: &Path, max_bytes: u64) -> Vec<String> {
    let Ok(mut f) = fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = f.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    let start = len.saturating_sub(max_bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // 半行
    }
    lines
}

/// 尾部窗口内的一条「有意义事件」。
enum Ev {
    /// 助手自然语言文字。
    Text(String),
    /// 工具调用。
    Tool(ToolDisplay),
    /// 工具结果（表示「刚跑完一次工具」，末尾若停在此仍算正在进行对应工具调用）。
    ToolResult,
}

/// 从尾部行序列计算「当前活动」。抽出便于单测（不触盘）。
fn analyze(kind: AgentKind, lines: &[String]) -> Option<Activity> {
    let mut evs: Vec<Ev> = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        push_events(kind, &v, &mut evs);
    }

    let mut last_text: Option<String> = None;
    let mut last_tool: Option<ToolDisplay> = None;
    // 最后一条「有意义事件」种类：决定末尾是否算工具调用。
    let mut tail_is_tool = false;
    for ev in &evs {
        match ev {
            Ev::Text(t) => {
                last_text = Some(t.clone());
                tail_is_tool = false;
            }
            Ev::Tool(td) => {
                last_tool = Some(td.clone());
                tail_is_tool = true;
            }
            Ev::ToolResult => {
                // 工具刚跑完、助手尚未回话：仍视为「正在做该工具」。
                tail_is_tool = true;
            }
        }
    }

    let text = last_text.map(|t| truncate(&t, MAX_ACTIVITY_TEXT_CHARS));
    let tool = if tail_is_tool { last_tool } else { None };
    if text.is_none() && tool.is_none() {
        return None;
    }
    // `at` 由 `resolve_activity` 从文件 mtime 填充；`analyze` 是纯函数不触盘。
    Some(Activity {
        text,
        tool,
        at: None,
    })
}

fn push_events(kind: AgentKind, v: &Value, out: &mut Vec<Ev>) {
    match kind {
        AgentKind::Cursor | AgentKind::Claude => push_events_msg(v, out),
        AgentKind::Codex => push_events_codex(v, out),
        AgentKind::Grok => push_events_grok(v, out),
    }
}

/// Cursor / Claude：一次 assistant 消息含 `content:[{type:text},{type:tool_use}]`；工具结果在 user 消息的
/// `content:[{type:tool_result}]`。文字仅取 assistant 行，工具结果不限角色。
fn push_events_msg(v: &Value, out: &mut Vec<Ev>) {
    let is_assistant = v.get("role").and_then(|r| r.as_str()) == Some("assistant")
        || v.get("type").and_then(|t| t.as_str()) == Some("assistant");
    let content = v
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"));
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        return;
    };
    for item in arr {
        match item.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "text" if is_assistant => {
                if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                    let t = t.trim();
                    if !t.is_empty() {
                        out.push(Ev::Text(t.to_string()));
                    }
                }
            }
            "tool_use" => {
                let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("");
                out.push(Ev::Tool(classify_tool(name, item.get("input"))));
            }
            "tool_result" => out.push(Ev::ToolResult),
            _ => {}
        }
    }
}

/// Codex rollout：`response_item.payload` 的 `message`(assistant output_text) / `function_call` /
/// `function_call_output`；`event_msg.payload` 的 `agent_message`。reasoning / token_count 忽略。
fn push_events_codex(v: &Value, out: &mut Vec<Ev>) {
    let ttype = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let Some(payload) = v.get("payload") else {
        return;
    };
    let ptype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match (ttype, ptype) {
        ("response_item", "message") => {
            if payload.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                if let Some(t) = value_text(payload.get("content")) {
                    let t = t.trim();
                    if !t.is_empty() {
                        out.push(Ev::Text(t.to_string()));
                    }
                }
            }
        }
        ("response_item", "function_call") => {
            let name = payload.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let args = parse_args(payload.get("arguments"));
            out.push(Ev::Tool(classify_tool(name, args.as_ref())));
        }
        ("response_item", "function_call_output") => out.push(Ev::ToolResult),
        ("event_msg", "agent_message") => {
            if let Some(t) = payload.get("message").and_then(|m| m.as_str()) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(Ev::Text(t.to_string()));
                }
            }
        }
        _ => {}
    }
}

/// Grok：`{type:assistant, content, tool_calls:[{function:{name,arguments}}]}`；`{type:tool_result}`。
/// reasoning / user / system 忽略。
fn push_events_grok(v: &Value, out: &mut Vec<Ev>) {
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "assistant" => {
            if let Some(t) = value_text(v.get("content")) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(Ev::Text(t.to_string()));
                }
            }
            if let Some(arr) = v.get("tool_calls").and_then(|x| x.as_array()) {
                for tc in arr {
                    let func = tc.get("function");
                    let name = func
                        .and_then(|f| f.get("name"))
                        .or_else(|| tc.get("name"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    let args_val = func
                        .and_then(|f| f.get("arguments"))
                        .or_else(|| tc.get("arguments"));
                    let args = parse_args(args_val);
                    out.push(Ev::Tool(classify_tool(name, args.as_ref())));
                }
            }
        }
        "tool_result" => out.push(Ev::ToolResult),
        _ => {}
    }
}

// ── 工具归一化 ──

/// 归一化一次工具调用为展示信息。`args` 可为已解析对象或原始 JSON 字符串（内部再 `parse_args`
/// 兼容），供 transcript 尾部解析与 hook 实时上报（`report.rs`）共用。
pub(crate) fn classify_tool(name: &str, args: Option<&Value>) -> ToolDisplay {
    let parsed = parse_args(args);
    let args = parsed.as_ref();
    let lower = name.to_ascii_lowercase();
    let label = if is_run(&lower) {
        ToolLabel::Run
    } else if is_read(&lower) {
        ToolLabel::Read
    } else if is_write(&lower) {
        ToolLabel::Write
    } else {
        ToolLabel::Other(name.to_string())
    };
    let object = match &label {
        ToolLabel::Run => arg_command(args),
        ToolLabel::Read | ToolLabel::Write => arg_filename(args),
        ToolLabel::Other(_) => arg_generic(args),
    };
    ToolDisplay { label, object }
}

fn is_run(n: &str) -> bool {
    matches!(
        n,
        "bash" | "shell" | "run_terminal_cmd" | "local_shell" | "local_shell_call" | "exec" | "run"
    )
}

fn is_read(n: &str) -> bool {
    matches!(n, "read" | "read_file" | "view" | "cat")
}

fn is_write(n: &str) -> bool {
    matches!(
        n,
        "write"
            | "edit"
            | "multiedit"
            | "str_replace"
            | "str_replace_editor"
            | "str_replace_based_edit_tool"
            | "search_replace"
            | "apply_patch"
            | "create_file"
            | "write_file"
    )
}

/// 从参数取命令首段（`command` / `cmd`，数组则 join）。
fn arg_command(args: Option<&Value>) -> Option<String> {
    let o = args?;
    for k in ["command", "cmd"] {
        if let Some(s) = o.get(k).and_then(value_scalar_or_join) {
            if !s.trim().is_empty() {
                return Some(truncate(&s, MAX_TOOL_OBJECT_CHARS));
            }
        }
    }
    if let Some(s) = o.as_str() {
        return Some(truncate(s, MAX_TOOL_OBJECT_CHARS));
    }
    None
}

/// 从参数取文件名（路径末段）。
fn arg_filename(args: Option<&Value>) -> Option<String> {
    let o = args?;
    for k in [
        "path",
        "file_path",
        "target_file",
        "filename",
        "file",
        "notebook_path",
    ] {
        if let Some(s) = o.get(k).and_then(|v| v.as_str()) {
            let seg = s.trim_end_matches('/').rsplit('/').next().unwrap_or(s);
            let seg = if seg.is_empty() { s } else { seg };
            return Some(truncate(seg, MAX_TOOL_OBJECT_CHARS));
        }
    }
    None
}

/// 其它工具：取参数前一小段（先看整串，再看常见键）。
fn arg_generic(args: Option<&Value>) -> Option<String> {
    let o = args?;
    if let Some(s) = o.as_str() {
        if !s.trim().is_empty() {
            return Some(truncate(s, MAX_TOOL_OBJECT_CHARS));
        }
    }
    for k in [
        "query",
        "pattern",
        "q",
        "glob_pattern",
        "search_term",
        "description",
        "prompt",
        "url",
        "path",
        "file_path",
        "command",
    ] {
        if let Some(s) = o.get(k).and_then(value_scalar_or_join) {
            if !s.trim().is_empty() {
                return Some(truncate(&s, MAX_TOOL_OBJECT_CHARS));
            }
        }
    }
    None
}

// ── 通用小工具 ──

/// 参数值 `arguments` 常为 JSON 字符串（Codex/Grok）或对象（Cursor/Claude 的 input）。
/// 字符串则尝试解析成 JSON；解析失败保留原始字符串（供 `arg_generic` 兜底）。
fn parse_args(v: Option<&Value>) -> Option<Value> {
    match v {
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            Some(serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.to_string())))
        }
        Some(other) => Some(other.clone()),
        None => None,
    }
}

/// 标量或字符串数组 → 单个字符串（数组以空格连接）。
fn value_scalar_or_join(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

/// 提取文字：字符串，或数组 `[{text:"..."}]` / `["..."]`。
fn value_text(c: Option<&Value>) -> Option<String> {
    match c? {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    parts.push(s.to_string());
                } else if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(s.to_string());
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

/// 折叠空白并按字符数截断（超出补 `…`）。
fn truncate(s: &str, max: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max {
        let t: String = collapsed.chars().take(max).collect();
        format!("{}…", t.trim_end())
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cursor_text_plus_tool_run() {
        let ls = lines(&[
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":"先跑测试"},{"type":"tool_use","name":"Shell","input":{"command":"cargo test"}}]}}"#,
        ]);
        let a = analyze(AgentKind::Cursor, &ls).unwrap();
        assert_eq!(a.text.as_deref(), Some("先跑测试"));
        let tool = a.tool.unwrap();
        assert_eq!(tool.label, ToolLabel::Run);
        assert_eq!(tool.object.as_deref(), Some("cargo test"));
    }

    #[test]
    fn tail_text_after_tool_drops_tool() {
        // 工具之后又产出文字（最终答复）→ 只显示文字。
        let ls = lines(&[
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a/b/registry.rs"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"..."}]}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":"改好了"}]}}"#,
        ]);
        let a = analyze(AgentKind::Cursor, &ls).unwrap();
        assert_eq!(a.text.as_deref(), Some("改好了"));
        assert!(a.tool.is_none());
    }

    #[test]
    fn tool_result_tail_keeps_tool() {
        // 末尾停在 tool_result（工具刚跑完）→ 仍显示该工具。
        let ls = lines(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"读一下"},{"type":"tool_use","name":"Read","input":{"file_path":"src/agents/title.rs"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"..."}]}}"#,
        ]);
        let a = analyze(AgentKind::Claude, &ls).unwrap();
        assert_eq!(a.text.as_deref(), Some("读一下"));
        let tool = a.tool.unwrap();
        assert_eq!(tool.label, ToolLabel::Read);
        assert_eq!(tool.object.as_deref(), Some("title.rs"));
    }

    #[test]
    fn codex_function_call_shell() {
        let ls = lines(&[
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"运行一下"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls -la\"]}"}}"#,
        ]);
        let a = analyze(AgentKind::Codex, &ls).unwrap();
        assert_eq!(a.text.as_deref(), Some("运行一下"));
        let tool = a.tool.unwrap();
        assert_eq!(tool.label, ToolLabel::Run);
        assert_eq!(tool.object.as_deref(), Some("bash -lc ls -la"));
    }

    #[test]
    fn codex_agent_message_only() {
        let ls = lines(&[
            r#"{"type":"response_item","payload":{"type":"function_call_output","output":"done"}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"任务完成"}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
        ]);
        let a = analyze(AgentKind::Codex, &ls).unwrap();
        assert_eq!(a.text.as_deref(), Some("任务完成"));
        assert!(a.tool.is_none());
    }

    #[test]
    fn grok_text_plus_tool_read() {
        let ls = lines(&[
            r#"{"type":"assistant","content":"看看这个文件","tool_calls":[{"function":{"name":"read_file","arguments":"{\"path\":\"/x/y/registry.rs\"}"}}]}"#,
        ]);
        let a = analyze(AgentKind::Grok, &ls).unwrap();
        assert_eq!(a.text.as_deref(), Some("看看这个文件"));
        let tool = a.tool.unwrap();
        assert_eq!(tool.label, ToolLabel::Read);
        assert_eq!(tool.object.as_deref(), Some("registry.rs"));
    }

    #[test]
    fn other_tool_keeps_raw_name_and_arg() {
        let ls = lines(&[
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Grep","input":{"pattern":"AgentRecord"}}]}}"#,
        ]);
        let a = analyze(AgentKind::Cursor, &ls).unwrap();
        let tool = a.tool.unwrap();
        assert_eq!(tool.label, ToolLabel::Other("Grep".to_string()));
        assert_eq!(tool.object.as_deref(), Some("AgentRecord"));
    }

    #[test]
    fn nothing_meaningful_returns_none() {
        let ls = lines(&[
            r#"{"type":"reasoning","summary":"..."}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count"}}"#,
        ]);
        assert!(analyze(AgentKind::Codex, &ls).is_none());
    }

    #[test]
    fn text_truncated_to_limit() {
        let long = "字".repeat(600);
        let line = format!(
            r#"{{"role":"assistant","message":{{"content":[{{"type":"text","text":"{long}"}}]}}}}"#
        );
        let a = analyze(AgentKind::Cursor, &lines(&[&line])).unwrap();
        let t = a.text.unwrap();
        assert!(t.chars().count() <= MAX_ACTIVITY_TEXT_CHARS + 1);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn read_tail_drops_partial_first_line_and_keeps_tail() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.jsonl");
        let mut content = String::new();
        for i in 0..100 {
            content.push_str(&format!("line-{i}-{}\n", "x".repeat(50)));
        }
        std::fs::write(&f, &content).unwrap();
        let tail = read_tail(&f, 512);
        // 只取到尾部若干行，且不含最早的行。
        assert!(!tail.is_empty());
        assert!(tail.iter().all(|l| !l.starts_with("line-0-")));
        assert!(tail.iter().any(|l| l.starts_with("line-99-")));
    }
}
