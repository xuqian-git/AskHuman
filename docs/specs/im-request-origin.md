# IM 普通提问卡来源标题

> 状态：已定案并实现。仅适用于普通 AskHuman Message / Question 卡；结构化确认卡保持自己的领域标题。

## 目标

普通 IM 提问卡与 Popup 一样，让用户能立即看出请求来自哪个 source / Agent、属于哪个项目。来源上下文
必须按请求传递，不能从常驻 daemon 的进程环境重新推断。

## 标题规则

- 单题且没有共享 Message：`Question from Codex · HumanInLoop`。
- 单题且有共享 Message：Message 卡为 `Message from Codex · HumanInLoop`，问题卡为
  `Question · Codex · HumanInLoop`。
- 多题：Message 卡同上；每张问题卡分别为
  `Question 1/2 · Codex · HumanInLoop`、`Question 2/2 · Codex · HumanInLoop`。
- 自定义 source 与 Agent 不同：两者都显示，如
  `Message from MyAgent · Codex · HumanInLoop`；相同值不重复。
- source / Agent / 项目任一缺失时只省略该段，不留下空分隔符。项目只显示路径 basename。
- 现有 i18n 继续负责 `Message from` / `Question from` / `Question i/n`；Agent 和项目名不翻译。
- 飞书、Telegram、Slack、钉钉的初始卡与答后终态卡复用同一个标题字符串。

## 上下文与 MCP 时序

`ConversationOrigin { source, agent_label, project_name }` 独立于 `AskRequest`：它描述投放上下文，不是问题
内容。daemon 从每个 `RequestEntry` 的 `ShowPayload` 取得 source / project，并优先使用异步解析出的 Agent
家族；单进程回退从 `AppState` 构造同一模型。公共 `run_conversation` 统一组装 Message 和 Question 标题，
各渠道只负责渲染。

MCP 会清理 Agent 环境标记，daemon 需靠进程树异步解析。只有本次确实要投放 IM 且 CLI 未直接给出
Agent 时，IM 最多等待 200ms 复用该解析结果；Popup 已先行投放，不受这段等待影响。解析及时完成则用
真实 Agent 替换通用 `the Loop`，超时或解析失败则用已有 source + 项目降级，不能无限阻塞。

## 非目标

- 不改变权限审批、Stop confirmation、Stage 等结构化确认卡；它们已有工具 / Project 等领域上下文。
- 不改变问题正文、选项、附件、提交值、抢答或卡片终态行为。
- 不为 IM 增加可点击 Agent / 项目胶囊；这里只统一文本标题。
