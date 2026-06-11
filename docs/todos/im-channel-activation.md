# IM 渠道按「工作会话」自动激活 / 反激活 — 设计讨论记录

> 状态：**设计讨论中（方向已锁定，未开始实现）**。本文记录问题、方案演进（含被否决项及原因）、选定模型、Agent 信号对照、待定决策与实现落点，供新会话恢复进度。
>
> **选定模型见 §5**：三层信号 = 进程存活（电平骨干）+ turn-start↔turn-end 成对事件（判忙/闲）+ TTL（仅兜底）。

---

## 1. 背景与问题

AskHuman 支持多种「人机交互出口」：本地**弹窗（popup）** + 多种 **IM 渠道**（钉钉 / 飞书 / Telegram / Slack）。

**当前行为**：每次提问，daemon 会把**所有已启用的 IM 渠道**连同弹窗**全部并行发出**，由协调器「首个终态结果生效、其余收尾」（抢答模型）。

**痛点**：用户在电脑前用弹窗答题时，每道题也都推给了**每个** IM → IM 里堆积大量消息（刷屏）。用户希望：
- 在电脑前 → 基本只用弹窗，IM 不刷屏；
- 离开电脑 → 能在某个 IM 收到提问并回复。

---

## 2. 目标

- 在电脑前：默认只弹窗，IM **零刷屏**。
- 离开电脑：提问能可靠地送达某个 IM，并能在该 IM 回复。
- 多 IM 配置下，同一时刻最多只激活**一个** IM（避免多 IM 同时刷）。
- 切换要自然：换到哪用哪、回弹窗就停 IM。
- 不引入「多台电脑必须配不同 bot」这类重负担（见坑2）。

---

## 3. 现状（代码事实，实现时以此为准）

- 抢答协调器：`src-tauri/src/app/coordinator.rs`
  - `Coordinator`：并行 Channel，**首个终态结果生效**（`submit`），其余 `interrupt` 收尾后输出退出。
  - `register()` 登记渠道；`is_finalizing()` 标识收尾阶段。
- 渠道抽象：`src-tauri/src/channels/mod.rs`（`Channel` trait：`start`/`interrupt`）、`conversation.rs`（`MessagingChannel` + `run_conversation` 公共编排：发消息→逐题 `ask_question`→`submit`）。
- 各 IM 渠道：`channels/{dingding,feishu,telegram,slack}.rs`；底层长连接 Router：`{dingtalk,feishu,slack}/router.rs`、Telegram 在 `channels/telegram.rs` + 轮询。
- **每次提问的渠道挂接**：`src-tauri/src/daemon/mod.rs` 的 `attach_im_channels()`（约 L837）：
  - 读 `AppConfig::load()`，对 `is_dingding_active / is_feishu_active / is_telegram_active / is_slack_active` 为真者，`ensure_*_router()`（**懒建连 + 缓存到 `ServerState`**）→ `register` + `start`，全部并行。
  - 这就是「全发 → 刷屏」的根因。
- 提问主流程：`daemon::handle_submit()`（约 L533）：`attach_im_channels` + `spawn_gui_helper`（弹窗为独立短命进程）→ 等结果 / CLI 断开。
- 现有 Hook 接入：
  - Cursor：`src-tauri/src/integrations/cursor_hook.rs` —— 仅注册 **`preToolUse`** 一个钩子（命中 Shell 里的 `AskHuman` 调用时把 timeout 抬到 24h，防等待用户时被取消）。脚本 `askhuman-timeout.sh`。**当前未用任何会话生命周期事件。**
  - Claude：`src-tauri/src/integrations/claude_hook.rs`（同类思路）。
- 长连接平台约束（见 `docs/overview.md` Phase 2）：**同一 bot/app 同一时刻仅允许一条 Stream/长连接**（钉钉 Stream、飞书 WS、Slack Socket Mode、Telegram getUpdates 单 offset）。当前靠「每种全局仅一条长连接 + 懒建连、用完缓存」规避**单机内**多开互抢。

---

## 4. 方案演进（含被否决项与原因）

### 4.1 用户原始构想：粘性·最近活跃 + 在 IM 「声明」
模型：「弹窗常驻 + 单一活跃 IM 槽」。活跃槽默认空（只弹窗）；在某 IM 回复 / 发「here（我在这）」→ 该 IM 成为活跃槽；在弹窗回复 → 清空回只弹窗；每次新提问按当前活跃槽决定是否同时发该 IM。
- 优点：贴合直觉、精确可控、最多占一个 IM。

### 4.2 坑1：「人不在、又没声明」的首条提问会漏 —— 已解决
用户解法（优于定时升级）：**here 不只是切换，还把当前所有"在途未答"问题补推到该 IM**，并设为活跃（粘性）。于是不需要定时兜底。

### 4.3 坑2：要「空闲也能收 here」必须常驻长连接 —— 真问题
- 若为「空闲也能收 here」让 daemon **常驻** IM 长连接：多台电脑用**同一个 bot** 会持续抢连接（Telegram getUpdates 两端会把消息随机分走/互吞最明显）。→ 只能「每台机器配不同 bot」或「搭中心服务统一持连接转发」。用户的担心**正确**。

### 4.4 规避坑2的尝试：here 只在「有在途提问」时生效 —— 被否决
思路：有在途提问时 daemon 本就开着 IM 连接（按现状），那时**只监听不发卡片**即可收 here；无在途提问就不连。→ 不新增常驻连接，多机冲突范围＝与现在一致。
- **否决原因（用户指出）**：Agent 何时抛问题用户无法预知，所以无法掐准 here 的时机；发早了（还没在途问题）就被丢。**依赖用户主动声明本质不可靠。**

### 4.5 时间兜底升级（弹窗 N 秒没答 → 转发 IM）—— 被否决
- **否决原因（用户指出）**：超时时长很难定。回答一个问题本就可能要几分钟，若把 N 设成几分钟，则离开时 IM 要等几分钟才收到，延迟不可接受；设短又会在电脑前误升级。

### 4.6 选定方向：用「Agent 工作会话」门控 IM 监听（用户提出）
核心洞察：把「代价高的常驻长连接」**绑定到「此刻正在这台机器上工作」**。用户一般不会家里/公司同时跑 Agent → 同一 bot 不会有两条并发连接 → **抢占问题自然消失**。会话期内长连接常开 → 用户**随时**发 here 都生效（彻底摆脱"掐时机"）；不工作时不连。

### 4.7 弯路：「活动心跳 + 滑动 TTL」判忙——被否决
一度想把各家活动事件当**心跳**、最近 X 分钟有活动就算 armed、超时 disarm。
- **否决原因（用户指出）**：Agent 跑长编译（30min–1h）期间**没有任何事件**，纯心跳时间戳会把"编译中(忙)"误判成"没人(闲)"，要求把 TTL 拉到 >1h，不可接受。

### 4.8 关键纠错：忙碌锁存同样会漏（用户指出）
把 `PreToolUse` 当"点亮忙"、等 `PostToolUse`"熄灭忙"——**和等 `sessionEnd` 是同一个坑**：end 事件一旦丢失（崩溃/强杀/漏发），忙状态永久锁存 → 泄漏。
- **根因原则**：事件都是**边沿(edge)信号**，会丢；任何"靠 start 事件点亮、靠 end 事件熄灭"的状态都可能漏关。
- **唯一不漏的是电平(level)信号**：轮询"那个东西此刻到底还在不在"。对"会话是否还在"，这个电平就是 **会话进程是否存活**。

### 4.9 再纠错：TTL 不能判"新起点"，只能兜底（用户指出）
若用 turn-end 当"终点"，就必须有 turn-start 当"起点"，二者**成对**。
- **检测"是否又开始新一轮"（busy 恢复 / 你回到电脑前）必须靠 turn-start 事件，不能靠 TTL**——turn-end 到下次 turn-start 的间隔可任意长（你可能隔很久才回来回复），TTL 判断不了"有没有新起点"。
- **TTL 仅作兜底**（容忍偶发漏事件），绝不当主判据。
- 推论：**SessionStart 不够**——它一个会话只触发**一次**，给不了"每一轮的起点"。**必须用每轮的 turn-start 事件**（`UserPromptSubmit` / `beforeSubmitPrompt`）。

---

## 5. 选定模型：三层信号（电平骨干 + 成对事件判忙 + TTL 仅兜底）

三层各司其职、互为兜底：

1. **电平骨干 = 会话进程存活**（不漏、跨任意长间隔）
   - daemon 轮询"该会话的 Agent 进程是否还活着"：活着=会话还在(armed)；死了=会话结束(disarm)。
   - 解决"长时间离开 / turn 间隔很长"——进程还活着就一直在，**不靠任何计时器**（§4.9）。
   - 解决"崩溃 / 直接关窗"——进程没了，轮询发现即收尾（§4.8）。

2. **成对事件判"忙/闲"（即"在不在电脑前工作"）**
   - **turn-start**（`UserPromptSubmit` / `beforeSubmitPrompt`）：你在本机敲了下一句 → 在电脑前 / 进入忙；可用于**反激活**（回到只弹窗）。
   - **turn-end**（`Stop` / `stop` / `agent-turn-complete`）：Agent 交回控制权 → 转空闲、卡着等人。
   - 编译期 **turn 没结束**（卡在 Bash 工具里）→ 不会有 turn-end → 仍判忙 → **天然无编译误判**（§4.7 的正解）。

3. **TTL 仅兜底**：只用于容忍"偶发漏掉 turn-end / 某 Agent 事件不可靠（如 Cursor）"；**绝不**用来判断"有没有新一轮 / 会话是否结束"（§4.9）。

> 会话「结束」的判据 = 进程死亡（轮询）/ `SessionEnd`（有则快速通道）/ 你显式关闭——**都不是"时间到了"**。

---

## 6. Agent 生命周期信号对照（实现关键）

| Agent | 类型 | 进程粒度 | turn-start | turn-end | 备注 |
|---|---|---|---|---|---|
| Claude Code | CLI | **单会话** = `claude` 进程 ✓ | `UserPromptSubmit` | `Stop` | 事件齐全可靠，含可靠 `SessionEnd`。最干净 |
| Codex | CLI/TUI | **单会话** = `codex` 进程 ✓ | `UserPromptSubmit`(v0.116+) | `Stop` / legacy `notify`(agent-turn-complete) | hooks 为**实验性**，需 `codex --enable codex_hooks` + `/hooks` 信任；**无原生 SessionEnd**（本模型下不影响）|
| Cursor | IDE | **整个 Cursor.app**（粗）✗ | `beforeSubmitPrompt` | `stop` | 有 sessionStart/End + tool hooks，但生命周期 hook 可靠性差 → **TTL 兜底权重更高** |

要点：
- **Codex 必须启用实验 hooks 才能拿到 `UserPromptSubmit`**：legacy `notify` **只有 turn-end、没有 turn-start** → 只用它就"拿不到起点、只能靠 TTL"，正中 §4.9 的坑。
- **Cursor 进程粒度粗**：能追到的 PID 是整个 IDE（开着就算活），不像 CLI 那样=单会话；故 Cursor 更依赖事件 + TTL 兜底，进程存活只能兜"整个 IDE 退出/崩溃"。
- **通用零 hook 兜底（PPID-at-ask）**：AskHuman 被 Agent 当子进程调用提问时，其父进程链上就是该会话的 Agent 进程；提问时上报该 PID 给 daemon 守活即可。⚠️ Agent 常经 `bash -c "AskHuman …"` 调用，直接 PPID 可能是临时 shell，需**向上 walk 进程树**找到稳定的 Agent 进程（codex/claude/cursor），而非裸取 PPID。这是"问过一次就能锁定会话进程"的弱兜底；hook(turn-start) 的价值是"在第一次提问之前就 arm，好让你更早发 here"。

---

## 7. 最终模型（目标行为）

- **不在工作**（进程不存在 / 已收尾）：IM 不连、不监听。
- **在工作**（会话进程存活）：各启用 IM 长连接常开，**默认只监听 here、不发卡片**（默认仍只弹窗 → 零刷屏）。
  - 任一 IM 发 **here**（会话期内随时，含长编译/长间隔）→ 把**所有在途未答**问题补推到该 IM + 设为**活跃槽（粘性、单个）**；此后**新**提问即时同时发该 IM（+弹窗）。
  - **turn-start / 弹窗回复** → 判定你在电脑前 → 清空活跃槽，回到「只弹窗」。
- 活跃槽**单个**：后发 here 的 IM 替换旧的。
- 会话结束（进程死亡 / `SessionEnd` / 显式关闭）→ 断开 IM、清空活跃槽。

> 注：默认「只监听不发卡片」相对现状是净减负——现状每个 IM 都发卡片；新模型默认 0 张、最多 1 张。

---

## 8. 关键约束与事实（务必记住）

1. 平台「同一 bot 同一时刻仅一条长连接」（钉钉 Stream / 飞书 WS / Slack Socket Mode / Telegram getUpdates 单 offset）。本方案靠「工作会话不跨机重叠」保证同一 bot 同时只有一条连接。
2. **唯一残留代价**：无 TTL 强制回收 → "忘了关但进程仍开着的空闲会话"会一直占着该 bot，直到进程真正退出（或 `SessionEnd` / 显式关闭）。跨机无中心服务做仲裁，自动让出要么靠 TTL（已否决当主判据）要么靠用户关掉旧会话。多机冲突仅在"两台机器同时有**活着的会话** + 同一 bot"时发生；用户认为同时**活跃**罕见、可接受。可选增强：检测疑似多机同 bot 并提示。
3. 信号三层职责（见 §5）：进程存活 = 电平骨干（判会话在不在）；turn-start ↔ turn-end 成对事件 = 判忙/闲（判在不在电脑前）；TTL = 仅兜底。
4. 需为各 Agent 装 turn 事件 hook 上报 daemon（见 §6 对照）；**Codex 需启用实验 hooks**（否则只有 turn-end、无 turn-start）。
5. 弹窗本地、免费、无刷屏顾虑 → **弹窗始终参与**；IM 是「可选、最多一个」的附加出口。

---

## 9. 待定决策（下次继续，逐项敲）

**已定**：方向 = §5 三层信号模型（电平骨干 + 成对事件判忙 + TTL 仅兜底）；信号原则见 §4.8 / §4.9。

**仍待定**：
- [ ] **TTL 兜底时长**：仅作容错的兜底值（漏 turn-end / Cursor 不可靠时多久判空闲）。**只兜底，不当主判据**。
- [ ] **here 指令形式**：固定关键词（`here` / `/here` / `我在`）大小写 / 多语言？是否任意消息即视为 here？建议显式关键词避免误触。
- [ ] **活跃槽粒度**：全局（每 daemon 一个）vs 按项目（project key 见 history）。建议先全局。
- [ ] **活跃 IM 数量**：固定单个（推荐）vs 允许多个。
- [ ] **turn hook 覆盖范围**：先 Claude + Codex（CLI、干净）？Cursor 何时纳入（可靠性差）。各接哪些事件作 turn-start / turn-end。
- [ ] **进程追踪实现**：PPID-at-ask 的 walk 进程树规则（识别 codex/claude/cursor 进程名）；轮询间隔；跨平台（`kill -0` / OpenProcess）。
- [ ] **here 回执**：发 here 后 IM 是否回「已切到此处，共 N 条待答」。
- [ ] **弹窗 / turn-start "反激活"判定**：仅「弹窗提交答案」算，还是「弹窗取消」也算清空活跃槽；turn-start 是否同样触发反激活。
- [ ] **多机同 bot 检测**：是否加提示（可选增强）。
- [ ] **可选新特性（待定是否做）**：turn-end 且人不在 → 主动往 IM 推「该你回复了」通知（需定义"人不在"判定）。
- [ ] **并发提问语义**：here 推「所有在途」；活跃槽对「新提问」生效——确认 in-flight 与 new 的边界。

---

## 10. 实现落点（代码参考，便于动手时定位）

- **渠道挂接改造核心**：`daemon::attach_im_channels()`（`src-tauri/src/daemon/mod.rs` ~L837）。由「全发」改为：会话存活(armed)时为各启用 IM **建连/保持但默认不发卡片（只监听 here）**；仅活跃槽对应 IM 才走 `run_conversation` 发卡片。
- **会话 / 活跃槽状态**：放 `ServerState`（参考已有 `update: Mutex<UpdateSnapshot>`、router 缓存字段）。需：会话表（进程 PID + 忙/闲(turn 状态) + 最近事件时刻）、`active_im: Option<ChannelId>`、armed 计算（= 有存活会话）。
- **进程存活轮询 + disarm 后台任务**：`daemon::serve()` 内 spawn 周期任务（参考已有 24h 更新检查、15s 指纹监听写法）；轮询 PID 存活（`kill -0` / OpenProcess），死亡即收尾该会话；TTL 仅作兜底容错。
- **turn 事件上报**：新增 CLI 子命令（被各 Agent 的 turn-start / turn-end hook 调用）→ IPC 通知 daemon 更新忙/闲与会话 PID。`ClientMsg` / `ServerMsg`（`src-tauri/src/ipc/mod.rs`，参考 `ClientMsg::Detect`、`ServerMsg::UpdateState`）。
- **PPID-at-ask 兜底**：AskHuman 提问路径里取自身父进程链、walk 到稳定 Agent 进程，连同 submit 一起上报 PID（`daemon::handle_submit` / IPC）。
- **IM 入站监听 here**：各 Router（`{dingtalk,feishu,slack}/router.rs`、`channels/telegram.rs` 轮询）需在「无会话卡片」时也能把入站自由文本上交 daemon 判定是否 here（现状无会话的入站会被丢弃 → 新增 daemon 级入站钩子）。
- **「推送所有在途」**：枚举在途请求并对指定渠道补发——参考 `daemon/request.rs`（登记表 / Coordinator / `broadcast_to_guis`）+ `coordinator.register` + `Channel::start`。
- **反激活（turn-start / 弹窗回复）→ 清活跃槽**：结果汇聚点（`Coordinator::submit` / `finish`，`coordinator.rs`）或 daemon 收到 popup 答案处，按 `source_channel_id == "popup"`；turn-start 上报时亦清。
- **Hook 安装扩展**：`integrations/cursor_hook.rs` / `claude_hook.rs` 增 turn 事件条目（CST 保留格式编辑）；**新增 Codex hook 安装**（`~/.codex/hooks.json`，需提示启用实验 hooks）。设置页开关 `src/views/SettingsView.vue`（已有 cursor/claude hook 安装项，Codex 目前只有 rules）。

---

## 11. 相关文件清单（速查）

- `docs/overview.md` —— 架构总览（渠道 / daemon / 抢答 / Phase 2-3 目标）。
- `src-tauri/src/daemon/mod.rs` —— `serve` / `handle_submit` / `attach_im_channels` / `ServerState` / 后台任务。
- `src-tauri/src/app/coordinator.rs` —— 抢答协调、结果汇聚、收尾。
- `src-tauri/src/channels/{mod,conversation,dingding,feishu,telegram,slack,popup}.rs`。
- `src-tauri/src/{dingtalk,feishu,slack}/router.rs`、`.../stream.rs|ws.rs` —— 长连接与路由。
- `src-tauri/src/integrations/{cursor_hook,claude_hook}.rs` —— 现有 timeout hook 接入（待扩展 turn 事件 + 新增 Codex hook）。
- `src-tauri/src/integrations/agent_rules.rs` —— 三 Agent 规则文件落点（Cursor `.mdc` / Claude `CLAUDE.md` / Codex `~/.codex/AGENTS.md`）；Codex 目前仅有 rules、无 hook。
- `src-tauri/src/ipc/mod.rs` —— C↔D 消息协议。
- `src-tauri/src/config.rs` —— 渠道配置与 `is_*_active`。
- `src-tauri/src/paths.rs` —— 各 Agent 配置文件路径（含 `codex_agents_md` 等；Codex hooks.json 路径待新增）。
