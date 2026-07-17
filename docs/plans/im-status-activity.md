# /status 展示 agent「当前在做什么」（解析 transcript 尾部）

让 IM 的 `/status` 除「初始标题 + 状态」外，反映每个 agent **当前实际在做什么**：解析各家会话
transcript 的**尾部**，取「最后一段助手文字 +（若末尾是工具调用则附）该次工具调用」。多 agent 场景下
做成**两级 `/status`**，避免全局输出过长。

本计划为自包含实现方案；背景机制参见 `docs/specs/agent-lifecycle-tracking.md`、
`docs/plans/im-channel-activation.md`（`/status` 现状）。

## 0. 涉及文件总览

- `src-tauri/src/agents/registry.rs`：`AgentRecord` 增稳定数字编号 `seq`；`Inner` 增单调计数器与分配；
  `load()` 重排、`snapshot()` 暴露 `seq`。
- `src-tauri/src/agents/activity.rs`：**新增**。`resolve_activity(kind, session_id) -> Option<Activity>`：
  尾部读取 + 四家字段映射 + 工具归一化 + 截断。
- `src-tauri/src/agents/title.rs`：抽出/暴露「按 session_id 定位 transcript 文件」与 `find_file_recursive`
  为 `pub(super)`，供 `activity.rs` 复用（避免重复定位逻辑）。
- `src-tauri/src/agents/mod.rs`：`mod activity;` 导出。
- `src-tauri/src/autochannel.rs`：`Command::Status` 携带可选编号；`classify` 解析编号；全局行加 `[seq]`；
  新增 `status_detail_text`；活动行/类别词的 i18n 渲染；单测。
- `src-tauri/src/daemon/mod.rs`：`handle_inbound` 分派 `Command::Status(sel)`；`/help` 文案补 `/status <编号>`。
- `src-tauri/src/i18n.rs`、`src/i18n/{en,zh}.ts`：新增本地化键（详情/无活动/未找到/类别词/help）。
- `docs/overview.md`：更新 `/status` 一节（两级命令 + 编号 + 当前活动）。

## 1. 需求与定案（来自访谈，均为定案，非可选项）

- **展示规则**：只要助手在该会话里输出过文字，**永远显示「最后一段助手文字」**；若 transcript 末尾是
  工具调用（含「工具刚跑完、助手尚未回话」），则在文字之后**再附上该次工具调用的内容**；若末尾是文字
  输出，则只显示该段文字。
- **适用状态**：工作中 / 空闲 都显示；空闲显示其「最后一段文字」（等价于它最后做的）。
- **命令范围（本期）**：只做 IM `/status`；GUI 状态窗口后续再说。推送/订阅更新（有变化主动推）本期不做。
- **多 agent 的两级命令**：
  - 全局 `/status`：保持紧凑，每行 `[编号] 类型 — 标题（项目）`，仍按「工作中 / 空闲」分组；**不带活动**。
  - 聚焦 `/status <编号>`：展示该 agent 的「头部信息 + 当前活动」。反复发同一命令即可查看是否有更新。
- **编号**：一个**自增稳定数字 ID**，**至少在当前 daemon 生命周期内不变**、纯数字、尽量短、便于输入。
- **可寻址范围**：只要注册表里**有记录**（工作中 / 空闲 / 已结束）都能用 `/status <编号>` 查看。
- **工具措辞**：只对**常见工具**归一化为中文类别词——**读取文件 / 写入文件 / 运行命令**；其余工具显示
  **原始工具名 + 参数前一小段**。每条工具调用前加符号 `▸` 标示「这是一次工具调用」。
- **截断**：详情里「最后一段助手文字」上限 **500 字**；工具对象（文件名 / 命令首段 / 参数前段）上限约 60 字。

## 2. 稳定数字编号 `seq`（registry）

- `AgentRecord` 增字段 `pub seq: u64`（`#[serde(default)]`，camelCase 序列化为 `seq`）。
- `Inner` 增 `next_seq: u64`（初始 1）与内部分配器 `alloc_seq(&mut Inner) -> u64`（返回当前值并自增）。
- **分配时机**：所有「新建记录」处各分配一次——`apply_event` 的 `None`（新建）分支、`upsert_working` 的
  `else`（新建）分支。已存在记录不改 `seq`（其生命周期内稳定）。
- **`load()` 还原**：盘上旧 `seq` 一律忽略（default 0）；对还原后的 active、ended 记录**按序重新分配** `seq`，
  并把 `next_seq` 设为已分配最大值 +1。→ 满足「当前 daemon 生命周期内稳定、从 1 起、纯数字、尽量短」；
  跨 daemon 重启会重排（用户已接受「至少 daemon 生命周期内不变」）。
- **snapshot()**：`AgentRecord` 序列化即带 `seq`，供 `autochannel` 渲染 `[seq]` 与按 `seq` 寻址。
- 单调、不复用：某记录结束后其 `seq` 不回收；已结束记录不进全局列表，但仍可被 `/status <编号>` 命中。

## 3. 活动解析 `agents/activity.rs`（新增）

### 3.1 文件定位（复用 title.rs）

在 `title.rs` 把下列能力提为 `pub(super)` 供复用：`find_file_recursive`，以及各家「按 session_id
定位 transcript 文件路径」的逻辑（可抽成 `pub(super) fn transcript_path(kind, sid) -> Option<PathBuf>`）：

- Cursor：`~/.cursor/projects/*/agent-transcripts/<sid>/<sid>.jsonl`
- Codex：`~/.codex/sessions/**/rollout-*-<sid>.jsonl`（`find_file_recursive` 后缀 `-<sid>.jsonl`）
- Claude：`~/.claude/projects/*/<sid>.jsonl`（`find_file_recursive` 目标 `<sid>.jsonl`）
- Grok：遍历 `~/.grok/sessions/*/<sid>/chat_history.jsonl`

### 3.2 尾部读取

`read_tail(path, max_bytes) -> Vec<String>`（`max_bytes = 256 KiB`）：seek 到文件末尾、读末尾 `max_bytes`、
utf-8 lossy 解码、按行切分、**丢弃首个可能被截断的半行**。与现有 `title.rs`「从头扫 `MAX_LINES` 行」方向
相反——活动要的是**最新**事件，且对超大 transcript 有界，不拖慢 daemon。

### 3.3 提取规则

在尾部窗口内逐行 `parse` JSON（失败跳过），得到窗口内**有序事件序列**，再计算：

- `last_text: Option<String>`：从后往前第一条「助手自然语言文字」。
- `tail_tool: Option<ToolDisplay>`：判断「最后一条**有意义**事件」是否工具调用——**有意义事件**限
  「助手文字 / 工具调用 / 工具结果」，忽略 reasoning、token_count、mode/permission 等元记录；末尾若是
  「工具结果」也视为「仍在进行该次工具调用」→ 取其对应工具调用。若末尾是助手文字（工具之后又产出文字，
  如最终答复）则 `tail_tool = None`。
- 组合（定案规则）：只要 `last_text` 存在就带上；`tail_tool` 存在再附工具行。

### 3.4 四家字段映射

助手文字 / 工具调用 / 工具结果 的识别：

- **Cursor**：`{role:"assistant", message.content:[{type:"text",text},{type:"tool_use",name,input}]}`；
  工具结果 `{type:"user"|role:"user", message.content:[{type:"tool_result"}]}`。
- **Claude**：`{type:"assistant", message.content:[{type:"text"},{type:"tool_use",name,input}]}`；
  工具结果 `{type:"user", message.content:[{type:"tool_result"}]}`。
- **Codex**：助手文字 `response_item.payload{type:"message", content:[{type:"output_text",text}]}` 或
  `event_msg.payload{type:"agent_message", message}`；工具调用 `response_item.payload{type:"function_call",
  name, arguments}`；工具结果 `payload.type=="function_call_output"`。Code Mode 从
  `custom_tool_call{name:"exec"}` 规范包装提取内层工具：常见只读 `exec_command` 显示为读取，其余命令
  保底显示为运行，`custom_tool_call_output` 闭合状态；`apply_patch` 则由
  `event_msg.payload.type=="patch_apply_end"` 表示完成/失败，`changes` 路径表显示为 `首文件名 +N`，
  且不重复展示 custom 调用；
  `event_msg.payload.type=="task_complete"` 视作回合结束（末尾按文字处理）；`reasoning` 加密/摘要，忽略。
- **Grok**：`{type:"assistant", content, tool_calls:[{function:{name, arguments}}]}`；工具结果
  `{type:"tool_result", tool_call_id, content}`；`{type:"reasoning"}` 忽略。

一次助手消息可能同时含文字与（一个或多个）工具调用；多工具时取**最后一个**（可在对象后标 `+N` 表示还有 N 个）。

### 3.5 工具归一化（定案）

只归一化常见工具为类别；其余保留原始工具名。对象取「文件名末段 / 命令首段 / 参数前段」，截断约 60 字：

- **运行命令**（类别 `Run`）：`Bash`、`Shell`、`shell`、`run_terminal_cmd`、`local_shell`、`exec`；
  对象 = 命令首段（`input.command` / `arguments.command`，数组则 join）。
- **读取文件**（类别 `Read`）：`Read`、`read_file`、`view`；对象 = 文件名（`input.path` / `file_path` /
  `target_file` 取末段）。
- **写入文件**（类别 `Write`）：`Write`、`Edit`、`MultiEdit`、`str_replace`/`search_replace`、`apply_patch`、
  `create_file`；对象 = 文件名。
- **其它**（类别 `Other(raw_name)`，含 Grep/Search/Glob/ask/Task/web 等）：显示**原始工具名** + 参数前一小段
  （从 `arguments`/`input` 取首个非空标量或整串前 ~40 字）。

所有工具行统一前缀符号 `▸`。

### 3.6 返回结构

```
struct Activity { text: Option<String>, tool: Option<ToolDisplay> }
struct ToolDisplay { label: ToolLabel, object: Option<String> }
enum ToolLabel { Run, Read, Write, Other(String /* raw tool name */) }
```

- resolver 只产出**结构化数据 + 已截断的内容**；类别词（运行命令/读取文件/写入文件）与前缀符号 `▸` 的
  **本地化渲染放在 `autochannel`**（与 `status_text` 同处、复用 i18n）。`object`/`Other(raw_name)` 是内容，不本地化。
- 常量：`MAX_ACTIVITY_TEXT_CHARS = 500`、`MAX_TOOL_OBJECT_CHARS = 60`；文字做空白折叠后截断（末尾补 `…`）。

## 4. 文本组装 `autochannel.rs`

- `Command::Status` 改为携带可选编号：`Status(Option<u64>)`（`Command` 仍可 `Copy`）。
- `classify`：命中 `/status`/`/状态` 后，取**第二个 token** 解析 `u64`——是数字 → `Status(Some(id))`；
  缺省或非数字 → `Status(None)`（全局）。
- 全局 `status_text`：`format_line` 前缀 `[seq]` → `[3] Cursor — <标题>（<项目>）`；分组与「工作中/空闲」不变。
- 新增 `status_detail_text(snapshot, id, lang)`：
  - 在 snapshot **全量**（active + ended）按 `seq == id` 找记录；找不到 → `statusDetailNotFound`（含 `{id}` +
    「发送 /status 查看列表」提示）。
  - 头部：`[id] 类型 — 标题（项目）· <状态词>`。
  - 调 `agents::activity::resolve_activity(kind, session_id)`：
    - 有 `text` → 追加一行（已 500 字截断）。
    - 有 `tool` → 追加 `▸ <类别词或原始工具名>: <对象>`（无对象则只显类别/原名，如询问类）。
    - 二者皆无 → `statusNoActivity`。
- 单测更新：`classify("/status")==Status(None)`、`classify("/status 3")==Status(Some(3))`、
  `classify("/status abc")==Status(None)`；`status_text` 含 `[seq]`；`status_detail_text` 命中/未命中/无活动。

## 5. daemon 接线 `daemon/mod.rs`

- `Parsed::Command(Command::Status(sel))`：`sel=None` → `status_text(snapshot)`；`sel=Some(id)` →
  `status_detail_text(snapshot, id)`。保留现有「`/status` 始终响应；auto 开且本次因 `/status` 切了活跃槽时
  附激活回执」逻辑，与是否带编号无关。
- `help_text`：`helpCmdStatus` 文案补充 `/status <编号>` 说明。

## 6. i18n 键（en / zh）

- `autoChannel.statusDetailNotFound`（含 `{id}`）
- `autoChannel.statusNoActivity`
- `autoChannel.activityRun` = 运行命令 / Run
- `autoChannel.activityRead` = 读取文件 / Read
- `autoChannel.activityWrite` = 写入文件 / Edit
- 状态词：`autoChannel.stateWorking`/`stateIdle`/`stateEnded`（详情头部用；或复用现有分组标题 + 新增 ended）
- `helpCmdStatus`：更新为含 `/status <编号>` 的说明
- 工具行前缀 `▸ ` 与 `Other(raw_name)` 的原始工具名在代码侧拼接，不入 i18n。

## 7. 边界与降级

- 全程 best-effort：定位失败 / 解析失败 / 无文字也无工具 → 详情回 `statusNoActivity`，全局列表照常（全局本就
  不带活动）。
- 尾部窗口 256 KiB 有界；详情为用户按需触发（非高频轮询），性能开销可忽略；活动**不缓存**（易变，每次现算），
  但会话文件定位可按需现查（量小）。
- 隐私/体积：只取文件名 / 命令首段 / 参数前段 + 助手文字（500 截断）；**不外泄工具结果正文**。
- Cursor 的工具**结果**可能不入 transcript，但工具**调用名**在（已核实），足够表达「正在做什么」。

## 8. 测试

- `activity.rs`：四家尾部样本 jsonl → 验证 `last_text` + `tail_tool` 判定 + 归一化（读/写/运行 + Other 原名）
  + 截断；`read_tail` 超窗口只取尾部并丢弃半行。
- `autochannel.rs`：`classify` 三态；`status_text` 带 `[seq]`；`status_detail_text` 命中/未命中/无活动。
- `registry.rs`：`seq` 单调分配；`load()` 重排且 `next_seq` 正确；`snapshot()` 带 `seq`。

## 9. 实现顺序

1. `registry`：`seq` 字段 + 分配器 + `load` 重排 + `snapshot` 暴露 + 单测。
2. `title.rs`：抽出/暴露文件定位 helper（`pub(super)`）。
3. `activity.rs`：`read_tail` + 四家解析 + 归一化 + 结构体 + 单测；`mod.rs` 导出。
4. `autochannel`：`classify` 扩展 + `[seq]` + `status_detail_text` + i18n 渲染 + 单测。
5. `daemon`：`handle_inbound` 分派 + `/help` 文案。
6. i18n：en/zh 键补齐。
7. `docs/overview.md` 更新；`install.sh` 编译验证。

## 10. 关联文档

- `docs/plans/im-channel-activation.md` —— `/status` 与活跃槽现状。
- `docs/specs/agent-lifecycle-tracking.md` —— 注册表 / 三态 / 会话身份。
- `src-tauri/src/agents/title.rs` —— 各家会话文件定位与 jsonl 解析（本方案尾部解析的复用来源）。
