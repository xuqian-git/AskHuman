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
        "HumanInLoop - Human-in-the-loop 交互工具".to_string(),
        String::new(),
        "用法:".to_string(),
        format!("  {prog} <message> [选项]      启动询问弹窗，结果写入 stdout"),
        format!("  {prog} --settings            启动设置界面"),
        format!("  {prog} --agent-help          显示面向 AI 的精简用法（提问相关）"),
        format!("  {prog} --help, -h            显示此帮助信息"),
        format!("  {prog} --version, -v         显示版本信息"),
        String::new(),
        "参数:".to_string(),
        "  <message>                      Message：所有问题的共享描述（可选；无 -q 时即作为唯一问题）".to_string(),
        String::new(),
        "选项:".to_string(),
        "  -q, --question <text>          声明一个实际问题，可多次出现（不写 -q 时 <message> 即唯一问题）".to_string(),
        "  -o, --option <text>            为「最近声明的问题」添加预定义选项；无 -q 时归 <message>；".to_string(),
        "                                 存在 -q 时不能出现在第一个 -q 之前；可多次出现".to_string(),
        "  -f, --file <path>              为 Message 附带展示文件（位置不限，可在 -q 之后），可多次出现".to_string(),
        "                                 （仅用于展示，不出现在结果输出中）".to_string(),
        "  --no-markdown                  关闭 Markdown 渲染（全局，默认开启）".to_string(),
        String::new(),
        "输出格式（成功路径）:".to_string(),
        "  单问题：[选择的选项] / [用户输入] / [图片] / [文件] 区块，仅在有内容时输出。".to_string(),
        "  多问题：每题以 # Qn 分组、题间用 --- 分隔；未答题输出「用户未回答此问题」。".to_string(),
        String::new(),
        "更多文档：参考应用内设置的「参考提示词」页面。".to_string(),
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
        "  <Message>             所有问题的共享描述（可选，默认按 Markdown 渲染）；".to_string(),
        "                        完全不写 -q 时它等价于 -q，即作为唯一问题".to_string(),
        "  -q, --question <text> 声明一个实际问题，可多次出现".to_string(),
        "  -o, --option <text>   为最近声明的问题加预定义选项；无 -q 时归 Message；".to_string(),
        "                        存在 -q 时不能出现在第一个 -q 之前；可多次出现".to_string(),
        "  -f, --file <path>     为 Message 附带文件或图片（绝对/相对/~，位置不限），可多次出现".to_string(),
        "  --no-markdown         关闭 Markdown 渲染（全局，对所有问题生效）".to_string(),
        String::new(),
        "用户回应（仅在有内容时返回，区块之间空行分隔）:".to_string(),
        "  [选择的选项]  用户勾选的预定义选项".to_string(),
        "  [用户输入]    用户输入的自由文本".to_string(),
        "  [图片]        用户附带图片的本地路径（可直接读取）".to_string(),
        "  [文件]        用户拖入的非图片文件本地路径（可直接读取）".to_string(),
        "  [状态]        用户取消时出现，请按其中说明继续询问".to_string(),
        String::new(),
        "多问题输出:".to_string(),
        "  每题以「# Qn」分组，题与题之间用「---」分隔；某题未作答时输出".to_string(),
        "  「[状态] 用户未回答此问题」；若所有题都未作答，则只输出一次取消提示。".to_string(),
        "  单问题时不加 # Qn 头，与既有格式一致。Message 仅作描述/附件展示，不进入输出。".to_string(),
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
