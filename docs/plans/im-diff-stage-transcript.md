# 实现计划：IM `/diff` · `/stage` · `/transcript`

> 需求：`docs/specs/im-diff-stage-transcript.md`（决策 D1–D23）。  
> 本计划描述**唯一方案**与落地顺序；不含「A 或 B」悬而未决项。

## 总览

| 模块 | 职责 |
|---|---|
| `autochannel` | 解析三命令 + help 文案 |
| `select` | `SelectAction`/`标题`/`PickerKind` 增 Diff·Stage·Transcript |
| `gitutil`（新） | 找 git 根、列 unstaged/untracked、生成 DiffModel、`git add -A` |
| `agents/transcript_full`（新） | 有界读 jsonl → TranscriptEvent 序列（四家 best-effort） |
| `render/diff_html` · `render/transcript_html`（新） | 自包含 HTML |
| `render/diff_docx` · `render/transcript_docx`（新） | 钉钉 docx（可基于 `dingtalk/docx` 扩展） |
| `confirm`（新） | ConfirmView 纯逻辑 + 台账字段定义 |
| 四渠道 confirm 渲染 | 飞书/TG/Slack P1；钉钉模板 P2 |
| `daemon` | 分派、选 agent、导出发送、stage 确认回调 |
| `i18n` | 全部用户可见文案 zh/en |

**分期**

- **P1**：命令解析 + 单选卡 + git/diff + transcript 解析渲染 + 飞书/TG/Slack Confirm；**飞书+钉钉 docx 附件**、TG/Slack HTML；钉钉 stage **文本降级（只预览列表，不执行 add）**。
- **P2**：钉钉 stage Confirm 模板 + 双按钮真正执行。

---

## P1-1 命令解析与 help

**触点**：`autochannel.rs`、`i18n.rs`

- `Command` 增：`Diff(Option<u64>)`、`Stage(Option<u64>)`、`Transcript(Option<u64>)`。
- `classify`：token `diff` / `stage` / `transcript`；第二 token 纯数字 → `Some`，否则 `None`。
- `help_text`：在 status/watch/msg 旁追加三行（`autoChannel.helpCmdDiff|Stage|Transcript`，含 `{p}`）。
- 单测：`classify` 覆盖有/无编号、`!` 前缀、非数字回落。

---

## P1-2 单选卡扩展

**触点**：`select.rs`、四渠道 select 渲染器、`daemon` 的 `PickerKind` / `send_agent_picker` / `handle_select_*`

- `SelectAction::{Diff, Stage, Transcript}`；按钮文案：`查看差异` / `暂存` / `会话`（en 对应）。
- 标题：`select.titleDiff|titleStage|titleTranscript`。
- 选项一律 `agent_options`（工作中+空闲），**不要**用 `watch_options`。
- 点选：
  - Diff → `run_diff(session_id)`，单选卡定格「已发送 diff」类文案（或空 ACK + 另发；**定稿：定格短文案 + 另发摘要与附件**，避免重复点击连发）。
  - Transcript → 同理。
  - Stage → **不** add；`start_stage_confirm(session_id)` 发 Confirm 卡；单选卡定格「已打开确认」。
- 钉钉单选卡已支持扩展 action 颜色：Diff/Transcript 用 default/blue，Stage 用 primary/blue。

---

## P1-3 `gitutil` 模块

**新文件**：`src-tauri/src/gitutil.rs`（daemon 侧同步 API，`std::process::Command` 调 `git`）

规则：

1. `find_git_root(cwd) -> Option<PathBuf>`：从 cwd 向上找 `.git`（文件或目录；兼容 worktree）。
2. `list_unstaged(root) -> Result<WorktreeChanges>`：
   - `git status --porcelain=v1 -uall` 解析；
   - 未 stage 的修改/删除：工作区相对 index 有 diff 的路径；
   - untracked：`??`；
   - **忽略**仅 index 相对 HEAD 的 staged 路径（ staged-only 不进列表）。
3. `build_diff_model(root, changes, limits) -> DiffModel`：
   - 已跟踪：`git diff -- <path>`（可 `--no-color`）；
   - untracked：读文件全文（UTF-8 lossy）；非 UTF-8/二进制 → binary 标记；
   - 单文件内容上限（如 200KB）超则 skip 内容；
   - 累计行数 5000 / 渲染前再检 HTML 2MB。
4. `stage_all(root) -> Result<StageResult>`：`git add -A`，返回成功路径数；stderr 失败映射。
5. 工作目录：所有 git 子进程 `current_dir = root`。
6. 单测：用临时目录 init repo fixture（改文件/untracked/staged-only）断言列表与 diff 边界。

截断策略（固定）：**文件列表完整列出（元数据）**；正文按文件顺序填充，超行数后后续文件只留「已截断」占位。

---

## P1-4 Diff 渲染

**新**：`src-tauri/src/render/diff_html.rs`（或 `agents` 旁 `export/` 目录，择一；推荐 `src-tauri/src/export/diff_html.rs` + `mod export`）

- 输入 `DiffModel` → 单文件 HTML 字符串：
  - 内联 CSS（红 `#ffebe9` / 绿 `#e6ffec` 行背景；等宽字体栈）；
  - 可选内嵌极简高亮（按扩展名关键字正则，无外网）；
  - 页头：agent 元信息、仓库路径、时间、截断警告。
- 钉钉：`export/diff_docx.rs`——按文件分节 + 等宽段落；行前缀 `+`/`-`/` `；可用红色/绿色 run（OOXML `w:color`）；复用 `dingtalk/docx` 的 package/esc 能力（抽公共或调用其内部助手若可见性允许——必要时在 `docx.rs` 增 `build_diff_docx` 入口）。

文件名：`diff-{seq}-{project_slug}.html`；project 取 git root 末段 sanitize。

---

## P1-5 Transcript 全量解析

**新**：`src-tauri/src/agents/transcript_full.rs`

- `load_events(kind, session_id) -> Result<TranscriptDoc, …>`：
  - 路径：`title::transcript_path`（需 `pub(crate)` 暴露）。
  - 读取：文件 ≤2MB 全读；更大则 **读尾部 2MB**（丢半行），页头 `truncated_from_start`。
  - 事件上限 2000：超出丢弃最旧。
- 四家族 `push_full_events`（与 `activity::push_events` 分离，避免拖垮 activity 语义）：
  - User / Assistant 文本；
  - Thinking/reasoning；
  - ToolCall + 后续 result 关联（id 能匹配则合并，否则顺序配对）；
  - Todo 类可降为普通 ToolCall 或弱展示；
  - 注入块（`<environment_context>` 等）跳过或折叠 Meta。
- `classify_askhuman(tool) -> Option<AskHumanBlock>`：
  - name/command 匹配 AskHuman CLI 或 MCP `ask`；
  - 从 args 抽 message/questions；
  - 从 result 抽 user_input / selected_options / status（兼容文本区块与 JSON）。
- 单测：各家族样例 jsonl fixture（可放 `src-tauri/tests/fixtures/transcripts/`）。

---

## P1-6 Transcript 渲染

- `export/transcript_html.rs`：
  - 聊天气泡布局（用户右/左或标签分区均可，固定一种：用户左灰、助手右白、系统弱化）；
  - Thinking：`<details>` 默认关闭；
  - Tool：概览一行 + `<details>` 参数/结果（结果截断如 2KB）；
  - AskHuman：边框强调块「🙋 向人类提问」+ 答复区；
  - 助手文本：简易 Markdown（可复用现有 markdown 能力若 Rust 侧有；否则轻量转义 + 代码块）。
- `export/transcript_docx.rs`：线性段落；思考用引用样式小节；工具用标题+代码；无 `<details>` 则默认只写概览+「详情见…」短摘要。

文件名：`transcript-{seq}-{title_slug}.html`；title 来自 registry，slug 截断 40 字符。

---

## P1-7 Confirm 抽象 + 飞书/TG/Slack

**新**：`src-tauri/src/confirm.rs`

- `ConfirmView { title, body, confirm_label, cancel_label }`
- `stage_confirm_view(lang, project, paths, total) -> ConfirmView`：body 为 markdown 列表前 30 路径 + 另有 N 个。

**daemon 台账**（仿 select，不持久化）：

```
ConfirmKind::Stage
ConfirmEntry { channel, message_id, session_id, git_root, paths_fp, created_at }
```

- TTL 30min；每渠道软上限；过期点击静默无效。
- `paths_fp`：排序后路径 join 的短哈希；确认时重列 unstaged，指纹不一致 → 文本「工作区已变化，请重新 /stage」且不定格成功。

**渲染**：

- 飞书 `feishu/card.rs::build_confirm_card` + `parse_confirm_action` → `(mid, ok|cancel)`。
- TG `telegram/confirm.rs`：HTML body + `confirm:ok` / `confirm:cancel` callback。
- Slack `slack/confirm.rs`：blocks + action_id。
- 路由：`ensure_confirm_routes` 与 select/watch 同构（P1 三渠道）。

**点选确认**：

1. ok → 重检路径 → `git add -A` → 编辑卡定格「已暂存 N 个文件」→ 可选再发文本路径摘要（**定稿：定格短结果 + 一条文本列最多 30 路径**）。
2. cancel → 定格「已取消暂存」。

---

## P1-8 daemon 分派与发送

**触点**：`daemon/mod.rs` `handle_inbound`

- `Command::Diff(sel)` / `Transcript(sel)` / `Stage(sel)`：
  - `None` → `send_agent_picker(PickerKind::…)`；
  - `Some(n)` → resolve by seq → 对应 `run_*`。
- `run_diff` / `run_transcript`：
  1. 解析 agent（cwd/kind/session/seq/title）；
  2. 构建模型 + 渲染字节；
  3. `reply_channel_text` 摘要；
  4. `send_channel_file(channel, path, name)`（抽公共：飞书/TG/Slack html；钉钉走 docx 字节上传）；
  5. 删临时文件。
- `run_stage`：只发 Confirm（钉钉 P1：见下）。
- 错误路径统一文本：无 cwd、无 git、无改动、无 transcript、git 失败、渲染/上传失败。

**钉钉 `/stage` P1 降级（写死）**：

- 列举将 stage 的路径，发**纯文本**：「以下文件可暂存（N）…；钉钉确认卡尚未启用，请在飞书/Telegram/Slack 执行 `/stage`，或稍后版本。」
- **不**执行 `git add`。

**发送摘要格式**（i18n）：

`[{seq}] {kind} · {project} · unstaged diff · {n} files`  
`[{seq}] {kind} · {project} · transcript`  
等。

---

## P1-9 i18n 与 overview

- `i18n.rs`：命令 help、空态、错误、确认卡、定格、摘要、截断说明（zh/en）。
- 完成后更新 `docs/overview.md`：IM 命令列表 + 模块表（gitutil/export/confirm/transcript_full）。
- `docs/PROGRESS.md` 跟踪本任务。

---

## P2 钉钉 stage Confirm 模板

- 新增 `docs/assets/dingtalk-confirm-card-template.json`（Markdown 正文 + 双 SingleButton + finalized 定格变量）。
- `dingtalk/confirm.rs`：`build_param_map` / `parse_confirm_action`。
- 配置项：默认 template id（与 watch/select 相同模式）。
- 接通后**删除** P1 文本降级，与其它渠道同语义执行 `git add -A`。

---

## 实现顺序（依赖）

```
P1-1 classify/help
  → P1-2 select 扩展
  → P1-3 gitutil（可与 P1-5 并行）
  → P1-4 diff 渲染
  → P1-5 transcript 解析
  → P1-6 transcript 渲染
  → P1-7 confirm + 三渠道
  → P1-8 daemon 串联 + 钉钉 docx 发送
  → P1-9 i18n/文档
P2 钉钉 confirm 模板
```

验证：`./scripts/install.sh` + 单测；真机四渠道抽测 diff/transcript；stage 在飞书完整走通确认/取消/指纹变化。

---

## 明确不做（对照 spec §7）

- commit/push/stash、partial stage、截图导出、PDF、钉钉 HTML、100% transcript 兼容。

## 用户定案摘要

见 spec 决策表 D1–D23；计划内无未决分支。钉钉 stage 确认卡按用户要求 **后置 P2**，P1 仅文本预览不执行 add。
