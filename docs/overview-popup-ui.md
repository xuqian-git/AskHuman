# Popup UI 概览

> 本文是 `docs/overview.md` 的专题补充，记录 Popup 当前实现地图；具体功能的需求与设计仍以对应 spec 为准。

## 窗口与附件交互

- 窗口拖拽用 `data-tauri-drag-region`（导航栏、底部空白和设置 tab 栏）；置顶用前端 `@tauri-apps/api/window` 的 `setAlwaysOnTop`。
- 文件拖入用 `onDragDropEvent` 取得原生路径；`-f` 附件拖出用 `tauri-plugin-drag` 的 `startDrag`。预览、系统图标和原生右键菜单由 `commands.rs` 中对应 command 提供。

## 来源标题与上下文

来源名（弹窗标题与渠道消息头共用）的解析优先级为 **自定义环境变量 `ASKHUMAN_ENV_SOURCE_NAME` > 探测到的发起 Agent 展示名（Claude Code/Codex/Cursor/Grok）> 默认「the Loop」**。后端入口为 `models::source_name_for_agent`；MCP 模式无法从 env 判断家族时回退默认名称。

当探测到 Agent 且未定制来源名时，`PopupView` 按 `popup.messageFrom/questionFrom` 的 `{source}` 占位把文案拆成前后两段，将 Agent 与 workspace 胶囊内联在标题中。未探测到 Agent 时仍显示默认来源；设置了自定义来源名时，标题使用自定义文本，胶囊继续作为上下文显示。窄窗下优先保留 Agent 名，再依次收缩项目名、标题前缀与后缀。

`.brand-time` 显示提问创建时刻的相对时间，满 24 小时后转绝对时间，hover 显示精确时间。时间锚点由 daemon `RequestRegistry::create()` 记录，经 `ShowPayload.created_at_ms` 和 `PopupInit.createdAtMs` 送到前端；冷弹窗和单进程路径以构造时刻兜底。

- **Agent badge**：来自 `AppState.agent_kind`。若 `PopupInit.agentTerminal` 表明对应终端可激活，badge 可调用 `focus_agent_terminal(agentPid)` 聚焦 Agent 终端。
- **workspace badge**：来自 `AppState.project`（git 根或 cwd），显示目录名、hover 展示完整路径，点击通过 `open_path` 在文件管理器打开。

这些字段通过 `PopupInit{project, projectName, agentKind, agentPid, agentTerminal}` 上送；预热 Popup 的上下文读取边界另见 `docs/specs/popup-prewarm.md`。

## 多问题纵向模式

设计见 `docs/specs/multi-question-vertical.md`，实现计划见 `docs/plans/multi-question-vertical.md`。该模式仅在 `experimental.verticalQuestions` 开启且问题数大于 1 时生效；关闭时保留一次一题的左右切换。

纵向模式由 `PopupView` 同时渲染所有题卡，以 active 指针统一键盘快捷键、语音和输入目标；滚动可更新 active，程序化导航期间有短暂锁定避免抖动。每题用 visited 状态跟踪是否看过，最后一题可见后才显示发送按钮。选项、文本、图片和回复文件均按题目索引保存；拖放图片按原生落点归属题卡，粘贴图片归当前聚焦题。单题不启用这些纵向模式样式与状态。

## 推荐选项

规格见 `docs/specs/recommended-option.md`。`-o!` / `--option!` 与普通选项语义相同，只增加“AI 推荐”标记；一题可有多个推荐项，但不会自动预选。Popup 与历史详情显示绿色推荐 badge，IM 渠道显示本地化推荐前缀；无论展示怎样变化，提交值始终恢复为原始选项文本。
