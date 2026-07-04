# /status 实时「当前工具」（扩展 Pre/PostToolUse hook）

`/status <编号>` 第二期。第一期（`docs/plans/im-status-activity.md`）靠解析各家 transcript **尾部**得到
「最后一段助手文字 +（末尾若是工具调用则附）该次工具」。但 **Cursor 的 transcript 要等一次工具执行完才落盘**
（一次 assistant 消息里的 `text`+`tool_use` 会在工具跑完后才写入 jsonl），于是「正在编译 / 正在等 AskHuman」
这类 **in-flight** 操作在详情里看不到，存在滞后。

本期用**已安装的 `PreToolUse`/`PostToolUse` lifecycle hook**（`__agent-hook <agent> activity`）在工具**开始时**就把
「当前工具」实时上报给 daemon，存进注册表；详情渲染时把它和 transcript 尾部结果**融合**，从而无滞后地反映
「此刻在跑什么」。这是通用方案，天然覆盖编译、长命令、等待人类回答等一切「工具执行中」场景。

本计划自包含；背景机制见 `docs/specs/agent-lifecycle-tracking.md`、
`docs/plans/im-status-activity.md`（一期）、`src-tauri/src/integrations/agent_lifecycle.rs`（hook 安装）。

## 1. 定案（来自访谈，均为定案，非可选项）

- **区分 pre/post（Q1 甲）**：现有 hook 命令对 `PreToolUse`/`PostToolUse` **都**上报 `activity`（不改）。由
  `report.rs` **解析 hook 经 stdin 传入的 JSON** 判断本次是 pre 还是 post（优先看显式的 hook 事件名字段，
  否则按「有无工具结果字段」启发式），并把工具信息随 `AgentEvent` 附带。**不改 hook 命令、无需 hook 迁移。**
- **覆盖范围（Q2 甲）**：四家统一（Claude / Codex / Cursor / Grok，均已装 Pre/PostToolUse hook）。**某家 stdin
  里取不到工具名 → 退化为纯心跳**（与今日一致，仅不显示实时工具，详情仍走 transcript）。
- **工具行取谁（Q3 甲）**：助手「最后一段文字」**永远取 transcript**；「末尾工具行」取
  **『注册表实时工具』与『transcript 尾部工具』里较新的一个**（比时间）。编译等 in-flight 场景 transcript 尚未
  落盘 → 显示实时工具；工具完成、transcript 追上后 → 回到 transcript。
- **清除时机 + 持久化（Q4 甲）**：`PostToolUse` / 回合结束 / 会话结束**都清**实时工具；**不持久化**到
  `agents.json`（仅内存 + snapshot，daemon 重启自然消失）；渲染侧**再兜底**——实时工具时间比 transcript 旧就弃用
  （防丢 post 导致残留）。
- **「最近动态」时间（Q4 追加）**：一期加的「最近动态（相对时间）」原本取 transcript 文件 mtime。本期规则：
  **当实际展示的是实时工具（它更新）时，相对时间用该工具的开始时间（hook 事件时间）**；否则仍用 transcript mtime。
  即时间戳始终对应「详情里实际展示的那条最新事件」。

## 2. 数据流总览

```
agent 要调用某工具
  └─(PreToolUse hook, stdin={tool_name,tool_input,...})→ AskHuman __agent-hook <agent> activity
        └─ report.rs: 解析 stdin → 判 phase=pre → classify 归一化 → ClientMsg::AgentEvent{..., tool:Some{name,object,phase:Pre}}
              └─ daemon: apply_event(心跳/置工作中) + registry.set_current_tool(session, name, object, at=now)
工具执行完
  └─(PostToolUse hook, stdin 含结果字段)→ ... report.rs 判 phase=post
              └─ daemon: registry.clear_current_tool(session)
/status <编号>
  └─ status_detail_text: transcript(text+tail_tool+mtime) ⊕ snapshot.currentTool → 取较新工具 + 对应时间
```

- `AgentEvent` 即发即走、旧 daemon 忽略未知字段（`#[serde(default)]`）→ 二进制新旧混用兼容。
- 全程 best-effort：取不到工具信息就当普通心跳，绝不影响既有生命周期追踪。

## 3. hook 上报侧 `agents/report.rs`

`report::run` 现已读取 stdin JSON（用于解析 `session_id`/`cwd`）。复用同一个 `Value`，当 `event == Activity` 时：

- **判 phase**：
  1. 优先读显式 hook 事件名字段：`hook_event_name` / `hookEventName`（Claude/Grok 为 `PreToolUse`/`PostToolUse`；
     Cursor 视其 stdin 是否带同类字段）。命中即定 pre/post。
  2. 否则启发式：stdin 含任一「结果字段」（`tool_response` / `tool_result` / `tool_output` / `response` /
     `output` / `function_call_output`，且非空）→ **post**；否则含 `tool_name`/`tool_input`/`tool_calls` → **pre**；
     两者皆无 → **无工具**（普通心跳，不带 tool）。
- **取工具名 + 输入**：名字段 `tool_name` / `toolName` / `tool`（字符串）；输入字段 `tool_input` / `toolInput` /
  `input` / `arguments`（对象或 JSON 字符串，复用一期 `parse_args`）。取不到名 → 不带 tool（退化心跳）。
- **归一化**：调 `activity::classify_tool(name, input)` 得到 `ToolDisplay{label, object}`；**过 IPC 只传原始
  工具名 `name` 与已截断的 `object`**（`label` 只由 `name` 决定，渲染侧再算，避免把不可序列化的枚举跨进程）。
- **组装**：`ClientMsg::AgentEvent{ ..., tool: Some(ToolReport{ name, object, phase }) }`（无工具时 `tool: None`）。
  事件时间由 daemon 收到时以本地 now 记（hook 与 daemon 同机、时差可忽略），无需 report 传时间。

> 注：Codex 的 hook stdin 结构与 Claude 不同（function-call 风格），字段名以实现时官方文档为准；取不到即退化。

## 4. IPC 扩展 `ipc/mod.rs`

`ClientMsg::AgentEvent` 增可选字段（`#[serde(default, skip_serializing_if = "Option::is_none")]`）：

```
tool: Option<ToolReport>
struct ToolReport { name: String, object: Option<String>, phase: ToolPhase }
enum ToolPhase { Pre, Post }   // serde rename_all = lowercase
```

旧 daemon 收到带 `tool` 的消息忽略该字段（`default`）；旧 CLI/report 不带 `tool` → `None`。向后/向前兼容。

## 5. registry 存储 `agents/registry.rs`

- `AgentRecord` 增 `current_tool: Option<CurrentTool>`：

```
struct CurrentTool { name: String, object: Option<String>, at: u64 }  // at = 上报时 daemon now（秒）
```

  **不落盘**：字段标 `#[serde(skip)]`（默认序列化既不进 `agents.json` 也不进 snapshot）；由 `snapshot()`
  **手动把内存里的 current_tool 注入返回的 Value**（键 `currentTool: {name, object, at}`）——snapshot 本就会
  逐条增补（如惰性标题），在此追加一处即可。→ 严格满足「仅内存 + 快照、重启消失」。

- 新增两个方法（按 `session_id`+`kind` 命中 active 记录；命中即刷新 `last_activity`）：
  - `set_current_tool(kind, session_id, pid, name, object) -> bool`：置 `current_tool=Some{name,object,at:now}`，
    并置 `state=Working`（pre 即在回合内）。返回是否状态变化（供广播）。
  - `clear_current_tool(kind, session_id)`：置 `current_tool=None`。
- `apply_event` 里 **`TurnEnd` / `SessionEnd` 分支顺带清 `current_tool`**（回合/会话结束不应残留在跑工具）。
  `SessionStart` 保持不动。
- daemon 侧 `AgentEvent` 处理：先照旧 `apply_event`（心跳 + 状态），再按 `tool` 分派：
  `Some(phase=Pre)` → `set_current_tool`；`Some(phase=Post)` → `clear_current_tool`；`None` → 不动。

## 6. 渲染融合 `autochannel.rs`

`status_detail_text` 现状：头部 + 空行 +「最近动态（<mtime 相对时间>）：」+ transcript 文字 + transcript 工具行。改为：

- 读 snapshot 记录里的 `currentTool`（若有）：`{name, object, at}`。
- 解析 transcript 得 `Activity{ text, tool(transcript), at(mtime) }`（一期已有）。
- **决定末尾工具行与展示时间**：
  - 令 `rt = currentTool`（实时）、`ts_tool = activity.tool`（transcript 工具）、`ts_at = activity.at`。
  - **实时优先条件**：`rt` 存在且 `rt.at > ts_at`（严格更新，代表 transcript 尚未追上）→ 展示工具 = 由
    `rt` 构造的 `ToolDisplay`（`label = classify_tool(&rt.name, None).label`，`object = rt.object`）；
    **展示时间 = `rt.at`**。
  - 否则 → 展示工具 = `ts_tool`；**展示时间 = `ts_at`**。
  - 助手文字**始终** = `activity.text`（transcript）。
- 「最近动态（<rel>）」的 `<rel>` 用上面选定的**展示时间**（`rel_time(now, 展示时间)`），而非固定 mtime。
- `render_tool` 复用一期实现（`▸ <类别/原名>: <对象>`）。
- 无 transcript 活动但有 `currentTool` 的情形（会话文件还没建/解析不到，但工具已在跑）：也应能显示——
  即「展示工具」可仅来自 `rt`，此时无助手文字，标题时间用 `rt.at`。

## 7. `agents/activity.rs` 复用点

- `classify_tool` 提为 `pub(crate)`（供 `report.rs` 归一化、`autochannel` 由 name 复得 label）。其依赖的
  `arg_command`/`arg_filename`/`arg_generic`/`parse_args`/`truncate` 保持私有（`classify_tool` 内部已封装）。
- `ToolDisplay` / `ToolLabel` 已 `pub`，跨模块可用；无需序列化（跨进程只走原始 name+object）。

## 8. 边界与降级

- 取不到工具名 / 未知家族字段 → `tool:None`，纯心跳，详情回落 transcript（与今日一致）。
- 丢 `PostToolUse`（异常退出等）→ `current_tool` 残留；靠**渲染侧兜底**（`rt.at <= ts_at` 即弃用）与
  回合/会话结束清除，避免长期显示过期工具。
- 隐私/体量：只过 IPC 传归一化后的 `name` + 短 `object`（≤60 字），**绝不传工具输入/结果正文**。
- 性能：hook 上报本就即发即走；registry 增两个 O(n) 查找（n 为活动 agent 数，量小）；snapshot 增一处注入。
- 时钟：hook 与 daemon 同机，`at` 用 daemon now；相对时间只到「秒/分钟/小时/天」，误差无感。

## 9. 涉及文件总览

- `agents/report.rs`：activity 事件解析 stdin 取工具名/输入 + 判 phase + 归一化 + 附 `tool` 到 `AgentEvent`。
- `ipc/mod.rs`：`AgentEvent` 增 `tool: Option<ToolReport>` + `ToolReport`/`ToolPhase` 类型。
- `client/mod.rs`：`report_agent_event` 透传新字段（若构造处需补字段）。
- `agents/registry.rs`：`AgentRecord.current_tool`（`serde(skip)`）+ `CurrentTool` + `set/clear_current_tool` +
  `apply_event` 的 turn-end/session-end 清除 + `snapshot()` 注入 `currentTool`。
- `daemon/mod.rs`：`AgentEvent` 处理按 `tool.phase` 调 set/clear。
- `agents/activity.rs`：`classify_tool` 提 `pub(crate)`。
- `autochannel.rs`：`status_detail_text` 融合实时/transcript 工具 + 展示时间；单测。
- `src/lib/types.ts`：`AgentRecord` 增可选 `currentTool?`（前端类型一致；GUI 暂不消费也无妨）。
- `docs/overview.md`：`/status` 一节补「实时当前工具（hook 上报）」。

## 10. 测试

- `report.rs`：给定 pre/post 各家样本 stdin JSON → 正确判 phase、取名/对象；无名样本 → 不带 tool。
- `registry.rs`：`set_current_tool` 置位 + 刷新活动；`clear_current_tool` / turn-end / session-end 清除；
  `snapshot()` 输出含 `currentTool`；`current_tool` 不进 `agents.json`（Persisted 序列化不含）。
- `autochannel.rs`：融合逻辑——`rt.at > ts_at` 用实时工具且时间取 `rt.at`；`rt.at <= ts_at` 用 transcript；
  仅 `rt` 无 transcript 时也能出工具行；助手文字恒取 transcript。

## 11. 实现顺序

1. `activity.rs`：`classify_tool` 提 `pub(crate)`（无行为变化）。
2. `ipc`：`AgentEvent.tool` + `ToolReport`/`ToolPhase`。
3. `report.rs`：解析 stdin 出工具名/输入 + phase + 归一化 + 附字段 + 单测。
4. `registry.rs`：`current_tool` 字段 + set/clear + turn/session 清除 + snapshot 注入 + 单测。
5. `daemon/mod.rs`：`AgentEvent` 按 phase 分派。
6. `autochannel.rs`：`status_detail_text` 融合 + 展示时间 + 单测。
7. `types.ts` + `docs/overview.md`；`install.sh` 编译验证。

## 12. 关联文档

- `docs/plans/im-status-activity.md` —— 一期（transcript 尾部解析、两级 `/status`、编号、最近动态）。
- `docs/specs/agent-lifecycle-tracking.md` —— 注册表 / 三态 / 会话身份 / hook 事件。
- `src-tauri/src/integrations/agent_lifecycle.rs` —— 四家 Pre/PostToolUse hook 安装与事件映射。
- `src-tauri/src/agents/report.rs` —— lifecycle 上报器（本期上报工具信息的入口）。
