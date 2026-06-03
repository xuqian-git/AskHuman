//! CLI 调用参考提示词（供设置界面展示与复制）。
//!
//! 用法片段直接复用 `--agent-help` 的输出（`help::agent_help_text`），
//! 保证「参考提示词」与命令实际用法始终一致；外层为行为约束规则。

/// 组装参考提示词：行为约束规则 + 内嵌 `--agent-help` 用法说明。
pub fn cli_reference() -> String {
    let usage = crate::cli::help::agent_help_text();
    format!(
        "- 必须通过 Shell 工具调用 `AskHuman` 命令对我进行询问，禁止直接询问或结束任务询问\n\
\n\
{usage}\n\
\n\
- 需求不明确时使用 `AskHuman` 询问澄清，提供预定义选项\n\
- 在有多个方案的时候，需要使用 `AskHuman` 询问，而不是自作主张\n\
- 在有方案/策略需要更新时，需要使用 `AskHuman` 询问，而不是自作主张\n\
- 即将完成请求前必须调用 `AskHuman` 请求反馈\n\
- 在没有明确通过使用 `AskHuman` 询问并得到可以完成任务/结束时，禁止主动结束对话/请求"
    )
}
