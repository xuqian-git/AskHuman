//! 帮助与版本文案。

use std::path::Path;

/// 用于帮助文案的程序名，取自 argv[0] 的 basename。
/// 为空或异常时回退到 "AskHuman"。这样任何包装器/软链/改名调用
/// 都会显示对应的名字，无需调用方额外配合。
pub fn program_name() -> String {
    program_name_from(std::env::args().next().as_deref())
}

fn program_name_from(arg0: Option<&str>) -> String {
    arg0.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            Path::new(s)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "AskHuman".to_string())
}

pub fn help_text() -> String {
    let prog = program_name();
    [
        "AskHuman - Human-In-The-Loop 交互工具".to_string(),
        String::new(),
        "用法:".to_string(),
        format!("  {prog} <message> [选项]       启动询问弹窗，参数见 --agent-help 说明"),
        format!("  {prog} --settings            启动设置界面"),
        format!("  {prog} --agent-help          显示面向 AI 的精简用法（提问相关）"),
        format!("  {prog} --help, -h            显示此帮助信息"),
        format!("  {prog} --version, -v         显示版本信息")
    ]
    .join("\n")
}

/// 面向 AI 的精简用法：仅含提问相关的调用方式、参数、结果区块与示例。
/// 既用于 `--agent-help`，也被「参考提示词」(prompts) 直接嵌入复用。
pub fn agent_help_text() -> String {
    let prog = program_name();
    [
        format!("{prog} —— 向人类发起提问并收集回应。"),
        String::new(),
        "调用方式:".to_string(),
        format!("  {prog} \"<Message>\" [-f \"<文件>\" ...] [-q \"<问题>\" [-o \"<选项>\" ...] ...] [--no-markdown]"),
        String::new(),
        "参数说明:".to_string(),
        "  <Message>             所有问题的共享描述（可选）；".to_string(),
        "  -f, --file <path>     为 Message 附带文件或图片（绝对/相对/~），可多次出现".to_string(),
        "  -q, --question <text> 提出问题，可多次出现，只有一个问题时可省略 -q".to_string(),
        "  -o, --option <text>   跟随在问题后，添加预定义回答选项".to_string(),
        "  --no-markdown         关闭 Markdown 渲染（对所有描述/问题生效）".to_string(),
        String::new(),
        "用户回应（仅在有内容时返回，空行分隔）:".to_string(),
        "  [选择的选项]  用户勾选的预定义选项".to_string(),
        "  [用户输入]    用户输入的自由文本".to_string(),
        "  [图片]        用户附带图片的本地路径（可直接读取）".to_string(),
        "  [文件]        用户拖入的非图片文件本地路径（可直接读取）".to_string(),
        "  [状态]        用户取消时出现，请按其中说明继续询问".to_string(),
        String::new(),
        "多问题输出:".to_string(),
        "  每题以「# Qn」分组，题与题之间用「---」分隔".to_string(),
        String::new(),
        "使用示例:".to_string(),
        format!("  {prog} \"要继续部署吗？\" -o \"继续\" -o \"停止\""),
        format!("  {prog} \"看看这个改动？\" -f ./diff.patch -q \"要继续吗？\" -o \"继续\" -o \"停止\""),
        format!("  {prog} \"以下是几处待确认\" -q \"保留日志？\" -o \"保留\" -o \"清除\" -q \"开启缓存？\" -o \"开\" -o \"关\""),
        format!("  {prog} \"纯文本内容（不渲染 Markdown）\" --no-markdown"),
    ]
    .join("\n")
}

pub fn version_text() -> String {
    format!("HumanInLoop v{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_basename_from_path() {
        assert_eq!(program_name_from(Some("/usr/local/bin/AskHuman")), "AskHuman");
        assert_eq!(program_name_from(Some("./AskHuman")), "AskHuman");
    }

    #[test]
    fn keeps_name_without_separator() {
        assert_eq!(program_name_from(Some("AskHuman")), "AskHuman");
        assert_eq!(program_name_from(Some("wblra ask")), "wblra ask");
    }

    #[test]
    fn falls_back_when_missing_or_empty() {
        assert_eq!(program_name_from(None), "AskHuman");
        assert_eq!(program_name_from(Some("")), "AskHuman");
        assert_eq!(program_name_from(Some("   ")), "AskHuman");
    }

    #[test]
    fn help_uses_program_name_and_default_is_unchanged() {
        let text = help_text();
        assert!(text.contains("<message>"));
        // Default (argv[0] basename) keeps the original "AskHuman" labeling
        // when not invoked under a different name.
        assert!(text.contains("HumanInLoop - Human-in-the-loop 交互工具"));
    }
}
