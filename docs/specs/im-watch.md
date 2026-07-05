# /watch 实时关注 agent 进度（IM 单卡静默刷新）

> 在 IM 里 `/watch <编号>` 关注一个 agent：机器人发一张「实时状态卡」，此后该 agent 的所有进度
> **就地更新这张卡**（编辑消息，不发新消息、零打扰）；卡片自带**动态按钮**（取消关注 / 立即刷新，
> 随状态变化，agent 结束后禁用并定格终态）。构建于既有生命周期追踪（`agents/registry.rs`）与
> `/status` 活动解析（`agents/activity.rs`）之上。

## 决策记录（用户经 AskHuman 定案）

- **D1 整体形态＝方案 A「纯静默单卡」**：进度只编辑卡片，**不**在关键转折发新消息。
  用户附加要求：卡片不是纯文本，下方要有**按钮**，且按钮随 watch 状态自动更新
  （如 agent turn 结束后按钮呈现不同状态）。
- **D2 钉钉路径＝专用模板**（用户后台新建 watch 模板），留待 P2；P1 不做钉钉。
- **D3 分期**：P1 仅飞书（完整交互 + 全部细节），Telegram / Slack / 钉钉后续批次。
- **D4 P1 交付方式**：设计写入文档后**直接实现**，不中途审核；完成后交用户真机验收再调。

## 渠道可行性（已查证）

| 渠道 | 就地更新机制 | 频控 / 时限 | 按钮 | 状态 |
|---|---|---|---|---|
| 飞书 | PATCH `im/v1/messages/:id`（代码已有 `patch_card`） | 单消息 5 QPS；应用 1000/min；仅 14 天内可改；卡片 ≤30KB | 卡片 button + callback（FsRouter 按 message_id 路由） | **P1** |
| Telegram | `editMessageText`（已有），bot 自己消息无时限 | ~1 次/秒/会话 | inline keyboard | P2 |
| Slack | `chat.update`（已有 `update_message`） | Tier 3 ≈50/min/频道 | Block Kit button | P2 |
| 钉钉 | `PUT /v1.0/card/instances` 更新公有变量（已有） | ~20 QPS/应用 | 模板内按钮 | P2（专用模板） |

数据粒度约束：数据源是 lifecycle hook（工具事件实时）+ transcript 尾部（Cursor 文字回合结束才落盘、
Codex 完成时整条写入、Claude 渐进写入），**没有 token 级流**。卡片是「变化驱动的最新一帧」，
不是逐字打字机。

## 命令面（`autochannel::classify` 扩展）

- `/watch <编号>`（别名 `/关注`）：关注该编号 agent（编号同 `/status`）。成功的回执**就是卡片本身**，
  不再回文本。重复 watch 同一 agent＝**换新卡**：旧卡定格为「已由新卡片接替」，发一张新卡
  （把卡拉到会话底部）。
- `/watch`（无参）：回**与 `/status` 相同的 agent 列表**（工作中/空闲 + 编号）+ 提示
  「发 /watch <编号> 关注」；已有关注时再附「正在关注」段（`[编号] 类型 — 标题（项目）· 状态`）。
  （验收反馈调整：原设计只列关注，改为先给可选对象、免用户再敲一次 /status。）
- `/unwatch <编号>`（别名 `/取消关注`）：取消该关注，旧卡定格「已取消关注」+ 回一条确认文本。
- `/unwatch`（无参）：恰有 1 个关注→取消它；多个→回列表让用户指定；0 个→提示。
- `/unwatch all`（`/unwatch 全部`）：全部取消。
- **渠道门控**：P1 仅飞书处理上述命令；其它渠道回「暂仅支持飞书」提示。`/help` 引导文案仅在
  飞书渠道列出 watch 命令。
- 关注上限：**5 个**（超出回提示）。

## 卡片（飞书卡片 JSON 2.0，`feishu/card.rs::build_watch_card`）

```
👁 实时关注 [3] · Cursor — HumanInLoop          ← 样式化头部行（icon + 蓝色小字）+ hr
🟢 工作中 · 已 6 分钟                             ← 状态行（emoji 状态 + 回合时长，见下）
「重构 daemon 空闲退出逻辑」                      ← 会话标题（有则显示）
最近动态（14:32:05）：                            ← 绝对时刻（本地时区），不用相对时间
我已经完成 registry 的改动，现在开始跑单测验证……
                                                 ← 文字与足迹之间空一行（用户定案：不用分隔线）
… 已省略 2 步                                     ← 灰字：文字之后超出 3 步窗口的更早调用数
● **读取**: *registry.rs*                        ← 足迹时间线：**最后一段文字之后**最近 ≤3 步
● **编辑**: *mod.rs*                                （旧→新），彩色圆点状态（已完成灰 / 进行中绿 /
● **运行命令**: *Rebuild after… (cargo build)*      失败红）；类别词加粗、参数斜体（去类别 emoji）
▸ 📋 TODO 4/7 · 当前：跑单测                      ← TODO 折叠面板（标题即摘要；默认收起，点开
                                                    见全清单；agent 未用 todo 功能则不出现）
────
最后更新 14:32:07                                 ← 灰色小字
[ 取消关注 ]  [ 立即刷新 ]                        ← column_set 两列按钮
```

- **状态行四态**：`🟢 工作中` / `⚪ 空闲` / `🙋 正在等待你的回答`（该 agent 有在途 AskHuman 提问时
  覆盖显示，优先级最高）/ `⏹ 已结束`。
- **回合时长（验收反馈新增；步数已按用户定案移除显示）**：工作中/等待回答时附 `· 已 X`——
  时长自 turn-start 起算（<1 分钟不显示；**不入签名**，只随其它变化顺带刷新）。依赖生命周期
  hook，未装则缺省。registry 仍统计 `turn_steps` 并注入 snapshot `turnSteps`（暂无展示消费方）。
- **足迹时间线（验收反馈新增，替代单工具行）**：**最后一段助手文字之后**的最近 ≤3 次工具调用
  （旧→新）——最新事件是文字时不显示任何工具行（用户定案：文字之前的调用属于上一段叙述）；
  文字之后超出 3 步窗口的更早调用以灰字「… 已省略 N 步」标注在时间线上方（计数入签名）。
  每步 = 状态圆点 + **类别词加粗** + *参数斜体*（用户定案：不用类别 emoji）。状态判定——
  **只有末步可能「进行中」**：新调用/助手文字的出现即证明前面的步已结束（Cursor 等家族不一定
  写工具结果事件，「后面又发生了事」是唯一可靠的完成信号）；带 `is_error` 的工具结果 → 失败
  （红点）。**Cursor 家族例外**（验收反馈修正「答完问题 AskHuman 仍绿点」）：Cursor 的
  transcript 只在工具**跑完后**才落盘该次调用（实测 in-flight 探针不可见）且从不写 tool_result
  ——落盘即已结束，故 Cursor 的 transcript 步一律「已完成」，「进行中」只能由实时 hook 的
  `currentTool` 并入（Claude/Codex/Grok 在调用开始时即写盘，末步无结果 = 真在跑，维持原判定）。
  hook 实时 `currentTool` 严格更新时并入为进行中末步。飞书卡用
  `<font color='green|grey|red'>●</font>` 彩色圆点；`/status`（纯文本，全渠道）用 emoji 圆点
  🟢/⚪/🔴（无粗斜体）。AskHuman 提问 = 一次 Shell 调用：等待时绿点，答完变灰点 + 状态行
  🙋→🟢 + 步数 +1，三处可感知。Cursor/Claude 的 Shell 调用自带人话 `description` → 显示
  `描述 (命令)`。`/status <编号>` 同享同一份解析。
- **TODO 清单（用户定案 A+B：摘要常显 + 折叠面板全清单）**：解析 transcript 里 agent 自报的
  任务清单——Cursor/Claude 的 `TodoWrite`（Cursor 带 `merge=true` 增量更新，按 id 就地重放合并；
  Claude 恒整表替换）、Codex 的 `update_plan`（整表）；Grok 无此机制。这些调用**不入**足迹
  时间线（实时 hook 侧同样过滤）。cancelled 条目剔除；条目文字截断 60 字符。飞书卡上渲染为
  **折叠面板**（collapsible_panel，默认收起不占高度，上外边距 12px 与足迹时间线拉开——用户
  定案）：标题即摘要行 `📋 TODO 4/7 · 当前：xxx`（「TODO」不翻译，用户定案）
  （done/total 计数 + 首个进行中条目；无进行中省略「当前」段），展开见全清单（进行中绿点加粗 /
  已完成灰点删除线 / 待办空心圈 ○）。清单变化计入签名（触发编辑；PATCH 会重置收起态，可接受）。
  `/status <编号>`（纯文本，全渠道）仅附摘要行。限制：解析窗口为 transcript 尾部 256KB，
  太久未更新的清单可能超窗丢失（best-effort）。
- **绝对时间**：卡片只在内容变化时编辑，相对时间会走字失真，故一律绝对时刻（今日 `HH:MM:SS`，
  跨日 `MM-DD HH:MM`，本地时区经 `libc::localtime_r`）。
- **动态按钮（D1 附加要求）**：
  - 活动态（工作中 / 空闲 / 等待回答）：`[取消关注]`（danger）+ `[立即刷新]`（default），
    behaviors callback `value={watch:"unwatch"|"refresh"}`。
  - 已结束：单个禁用按钮 `已结束 · 已自动取消关注`（结束即自动退订，卡片定格）。
  - 已取消 / 已换新卡 / 已跟底：单个禁用按钮 `已取消关注` / `已由新卡片接替` / `已移至最新卡片 ⬇`。
- 按钮回调经 FsRouter 卡片路由（watch 子系统注册一条 `RoutedFs`、`set_active(message_id)`），
  同步回 `callback_update_card(新卡)`——点击 Loading 直接变终态，无闪烁（复用 ask 卡机制）。
  - `unwatch`：ack 即回定格卡，同时移除订阅、持久化。
  - `refresh`：ack 即回按当前状态重算的新帧（并刷新签名）。

## 更新引擎（daemon 内）

- **触发**：① `AgentEvent`（turn/tool/会话结束）即时 `Notify` 唤醒；② 周期 tick——有「工作中」
  订阅时 2s、只有空闲订阅时 10s、无订阅时纯等 Notify；③ 提问创建 / 答复完成也 Notify（等待回答
  状态即时进卡）。
- **防刷**：每帧算**签名** =（状态、标题、最近文字、工具行、等待标志）——**不含活动时刻**：
  `at` 来自 transcript mtime / 工具心跳，会在内容不变时走动（如 Claude 渐进写盘），计入会造成
  「内容没变、卡片却被编辑（只有底部时间在跳）」的无谓更新（验收反馈修正）。签名不变不编辑；
  每卡最短编辑间隔 1s（tick 本身 ≥2s，远低于飞书单消息 5 QPS）。编辑失败记日志、下一拍全量重渲染
  重试（帧是全量的，丢帧无损）；**连续 5 次失败**（如超 14 天 / 卡被删）自动移除订阅。
- **机制说明（轮询 vs filewatch）**：引擎是「事件驱动为主 + 有界轮询兜底」的混合——hook 事件
  （turn/工具/提问/答复）即时 Notify，不等 tick；tick 只对**被关注的 ≤5 个** agent 读 transcript
  尾部（256KiB tail read），2s/10s 自适应，成本可忽略。transcript filewatch 暂不引入：四家
  transcript 路径形态各异且会轮转/新建（需盯目录 + 重挂），而 mtime 变化本就不等于展示内容变化
  （仍要读尾部算签名），watcher 只省掉「无变化时的 tail read」，收益小于复杂度。
- **跟底重发（验收反馈新增）**：watch 卡被会话里的**非 watch 消息**（用户消息 / 机器人文本回执 /
  提问卡）顶上去后，用户会找不到卡。机制：daemon 记一条渠道级**淹没水位线** `disturb_ms`
  （上述消息发生时间，Unix 毫秒；watch 卡自身的发送 / 编辑**不**计入——watch 卡之间互不影响、
  无级联），每张卡记 `sent_at_ms`。下一次**内容变化**时若 `disturb_ms > sent_at_ms`（已淹没）→
  不 PATCH 旧卡，改为**发一张新卡到会话底部**接续，旧卡定格为禁用按钮「已移至最新卡片 ⬇」；
  订阅换绑新 message_id（持久化 + 回调路由重建）。约束：
  - **节流 30s**（用户定案）：同一订阅两次跟底至少间隔 30s，窗口内的变化仍就地 PATCH（不丢内容）。
  - **提问期间抑制**：该渠道有在途 AskHuman 提问时不跟底（不打断问答会话），只就地 PATCH。
  - **答复完结豁免**：飞书参与的提问完结后清零全部订阅的节流（`last_move_ms=0`）——下一次内容
    变化**立即**跟底，用户答完马上能在底部看到回答引发的更新（用户定案）。**注意**：完结本身
    不标记扰动——作答/取消只是就地 PATCH 提问卡、不产生新消息；淹没判定完全依据提问卡发出时刻
    （attach 处标记）。否则提问期间新 /watch 的卡（已在底部）会被误判淹没而重发出连续两张卡
    （验收反馈修正）。
  - 内容不变仍不动卡（跟底也以内容变化为前提，纯静默原则不破）。
- **结束语义**：注册表里该 session 变 `ended`（或记录彻底消失）→ 最后一帧定格（⏹ + 禁用按钮）→
  自动退订、持久化。已淹没时结束帧也走跟底：新卡直接是终态卡，旧卡定格「已移至最新卡片」。
- **订阅持久化**：`~/.askhuman/state/watch.json`（`[{channel, sessionId, messageId, createdAt}]`，
  原子写）。daemon 重启 / 换新后**恢复并继续编辑同一张卡**；恢复时按 session_id 重解析显示编号
  （`seq` 不跨重启保留）。agent 的 hook 事件会 `ensure_running` 拉起 daemon，故重启后 watch 自动续命。
- **闲退守卫**：有活跃（未结束）watch 订阅时 daemon 不因空闲退出（订阅随 agent 结束而消亡，有界）。
- **路由韧性**：watch 回调路由任务随飞书 Router 生命周期重建（`ensure_inbound_listeners` 末尾 +
  新订阅时 + tick 发现路由缺失时幂等重挂）。

## 复用与新增触点

- 复用：`agents/activity.rs`（尾部解析 + 工具归一化）、`autochannel` 的实时工具融合（抽出
  `activity_parts`）、`agents/registry.rs::snapshot`（seq / 状态 / currentTool）、
  `feishu::client::patch_card/send_card`、`FsRouter::register/set_active`、
  `RequestRegistry::in_flight_agent_{session_ids,pids}`（等待回答判定）。
- 新增：`src-tauri/src/watch.rs`（纯逻辑：帧 / 签名 / 文案 / 持久化 / 本地时间格式化）、
  `feishu/card.rs::build_watch_card/parse_watch_action`、`daemon/mod.rs` watch 引擎 + 命令分派、
  `paths::watch_file`、i18n `watch.*`。

## 后续（不在 P1）

- P2：Telegram（editMessageText + inline 按钮）、Slack（chat.update + Block Kit）、
  钉钉（专用 watch 模板：markdown 变量 + 按钮，公有数据更新）。
- 可选：答完提问自动关注该 agent；`/watch` 关键转折附加通知的开关（用户当前明确选纯静默）。
