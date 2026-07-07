# 实现计划：/msg 选择卡 + 单选卡进行中抑制 Watch 跟底 + 插话「仅工作中」收敛

> 三块需求，用户已逐条定案（见文末「用户定案」）。基于既有 `select`（单选卡）、`agent-interject`
> （插话）、`im-watch`（实时关注）三套基础设施扩展，尽量复用。
>
> 相关既有 spec：`docs/specs/im-select-card.md`、`docs/specs/agent-interject.md`、`docs/specs/im-watch.md`。

## 需求 A：`/msg` 支持「选择项目卡」（无需先给编号）

目标：`/msg <内容>`（不带编号）也能发插话——本渠道「关注中」恰 1 个且工作中时直接发；否则弹一张
选择卡（每行一个「发送」按钮）让用户挑对象。

### A-1 解析（`autochannel.rs::classify`）

- `msg` 分支：首 token 是纯数字 → 仍按编号（现状不变）；**首 token 非数字 → 整段作为内容**，产出
  `Command::Msg(None, Some(内容))`（现状会丢内容变成 `Msg(None, None)`）。
- `/msg`（空）→ `Msg(None, None)`；`/msg <编号>` → `Msg(Some(n), None)`；`/msg <编号> <内容>` →
  `Msg(Some(n), Some(内容))`（均不变）。内容保留原始换行。
- 更新既有单测 `classify_msg_and_msg_clear`（`/msg hello` 现应为 `Msg(None, Some("hello"))`）。

### A-2 `select` 模块（`select.rs`）

- 新增 `SelectAction::Msg`；`button_label` 映射 `select.btnMsg`（「发送」/「Send」）。
- 新增 `pub fn msg_options(snapshot, watching, lang) -> Vec<SelectOption>`：**只列 `state=="working"`
  且 `kind!="grok"`** 的 agent（复用 `option_from_record`；`watching` 命中仍加「· 关注中」徽标）。
- 新增标题 `title_msg(lang)` → `select.titleMsg`（「选择要发送消息的 Agent」）。
- 单测：`msg_options` 排除 idle/ended/grok；`SelectAction::Msg.button_label` 文案。

### A-3 四渠道渲染器补 `Msg` 档

`SelectAction` 的穷尽 match 需补一臂（按钮样式），文案统一走 `button_label`：

- `feishu/card.rs::select_button_type`：`Msg => "primary"`。
- `dingtalk/select.rs::button_color`：`Msg => "blue"`（与 Watch/Status 同色）。
- `slack/select.rs::button_style`：`Msg => Some("primary")`。
- `telegram/select.rs`：无 match，仅用 `button_label`，无需改。

### A-4 daemon 台账与分派（`daemon/mod.rs`）

- `PickerKind` 增 `Msg`；`PickerEntry` 增字段 `payload: Option<String>`（待发送内容；其它 kind 为
  `None`）。`send_agent_picker` 增参 `payload: Option<String>` 并写入 `PickerEntry.payload`
  （三处既有调用传 `None`；`PickerKind::Msg => SelectAction::Msg`）。
- **`handle_msg_cmd` 重构**按 `(sel, content)` 四分支：
  - `(Some(id), Some(content))`：发送——`resolve_msg_target(require_working=true)`（见 A-6）→ `deliver_msg`。
  - `(Some(id), None)`：回显待送达（现状；`require_working=false`）。
  - `(None, Some(content))`：**自动流程 `handle_msg_auto`**：
    - 本渠道「关注中」session（`watching_sessions`）恰 1 个、且该 agent「工作中·非 grok」→ `deliver_msg` 直发。
    - 否则 → `msg_options` 组装工作中候选：为空回 `select.msgNoWorking`「当前没有工作中的 Agent，无法发送」；
      非空 → `send_agent_picker(kind=Msg, payload=content)`；发卡失败（非支持渠道）→ 文本兜底（列工作中 + 提示 `/msg <编号>`）。
  - `(None, None)`：**增强用法提示 `msg_usage_hint`**：`autoChannel.msgUsage` + 空行 + 当前工作中 agent 列表
    （`[编号] 类型 — 标题（项目）`，复用 `autochannel::kind_title_project`；无工作中则附一行 `select.msgNoWorking`）。
- **`deliver_msg(state, channel, session_id, content, config, lang)`**：`interject.append` → `persist` →
  `broadcast_agents_state` → 回执（`msgDeliveredNow` / `msgQueued{n}`）。抽出供直发与命令共用。
- **点选处理**（每渠道一臂，复用既有 watch 单选卡的变身/定格通道）：
  - 飞书 `handle_select_card_action` 增 `PickerKind::Msg => select_pick_msg(..., ack)`：
    重新校验目标「工作中·非 grok·存在」；失败 → ack 定格 `build_select_final_card(title_msg, select.msgTargetGone)` + 移除 picker；
    成功 → `deliver_msg`（回执另发文本）+ ack 定格 `build_select_final_card(title_msg, select.msgSentCard{id,note})` + 移除 picker；
    之后 `activate_channel_on_action`。
  - TG/Slack `dispatch_select_pick` 增 `PickerKind::Msg => select_pick_msg_inplace`：同上，收尾走 `finalize_select_card_edit`。
  - 钉钉 `handle_select_dd_action` 增 `PickerKind::Msg => dd_select_pick_msg`：同上，收尾走 `dd_finalize_select_card`。
  - 定格文案 `select.msgSentCard`：「已发送给 [{id}]（{note}）」，`{note}` = `msgDeliveredNow`/`msgQueued`；`{id}` 取快照 seq。
- i18n 新增：`select.titleMsg` / `select.btnMsg` / `select.msgSentCard` / `select.msgNoWorking` / `select.msgTargetGone`。

### A-6 显式编号发送「仅工作中」（`resolve_msg_target` 增 `require_working`）

- `resolve_msg_target(..., require_working: bool)`：`require_working` 为真且目标 `state!="working"` →
  回 `select.msgNoWorkingTarget`「该 Agent 当前空闲，只能给工作中的 Agent 发送」+ 返回 None。
- 调用点：`handle_msg_cmd` 发送分支传 `true`；回显分支传 `false`；`handle_msg_clear_cmd` 传 `false`（撤回不限工作中）。

## 需求 B：单选卡进行中，Watch 卡不「跟底」（复用提问期抑制机制）

现状：`watch_tick` 的跟底门 `move_ok = buried && !ask_active && 30s节流`；提问期间只就地编辑不跟底，
提问完结时清零 `last_move_ms` 立即跟底。把「有在途单选卡」也纳入抑制。

- 新增 `has_active_select_on(state, channel_id) -> bool`：该渠道存在 picker 即真。
- `watch_tick`：`let select_active = has_active_select_on(state, &ch);`，`move_ok` 追加 `&& !select_active`。
- **单选卡消费/移除即放开**：`remove_picker` 内清零该渠道全部 watch 订阅的 `last_move_ms=0` +
  `state.watch.notify.notify_one()`（用户定案：立即往下刷，与提问完结一致）。清零按 picker 实际渠道
  （含钉钉，覆盖到提问路径遗漏 dingding 的小坑）。TTL 过期清理（`register_picker` 内 retain）不特殊处理
  （罕见，下次内容变化按 30s 节流自然跟底）。

## 需求 C：状态窗口 + 状态栏菜单「只有工作中才能发消息」

插话只对「工作中」有意义（送达点是 agent 的下一次工具调用）。收敛入口：

- 状态窗口 `src/views/AgentsView.vue`：`canSendMessage(a)` 由 `kind!="grok" && state!="ended"` 改为
  `kind!="grok" && state==="working"`（idle 不再显示「发送消息」按钮）。
- 状态栏托盘 `app/gui_host.rs` 子菜单：「发送消息」条目门控由 `a.kind != "grok"` 改为
  `a.kind != "grok" && a.state == "working"`（`TrayAgentInfo.state` 已入 `menu_signature`，diff 生效）。
  idle agent 子菜单仅剩「聚焦终端」或退化为只读行。

## 验证

- `cargo test --manifest-path src-tauri/Cargo.toml` 全绿（含改动的 classify/select 单测）。
- `./scripts/install.sh` 编译进环境并**重启 daemon**（命令处理/卡渲染都在 daemon）。
- 真机（用户验收）：
  - `/msg 你好`：仅关注 1 个且工作中 → 直发回执；否则弹选择卡（列工作中·非 grok），点「发送」→ 定格「已发送给 [n]」。
  - 无工作中 agent 时 `/msg 你好` → 提示、不弹卡；`/msg` 空 → 用法提示 + 工作中列表。
  - `/msg <编号> <内容>` 对 idle 目标 → 拒绝提示；对工作中 → 正常。
  - 做单选期间 Watch 卡不往下刷；选完产生结果后立即往下刷。
  - 状态窗口 / 状态栏：idle agent 无「发送消息」。

## 用户定案（interview 结论）

- 直发条件基于「关注中」而非「工作中」：只有**明确关注恰 1 个**才直发，避免发错；否则一律弹卡。
- 卡片候选与所有发送对象一律限「工作中」，且**排除 grok**；无工作中对象则提示、不弹卡。
- 点「发送」后卡片就地定格「已发送给 [编号]」（带送达/排队条数）。
- 单选完成后立即放开 Watch 跟底（与提问完结一致）。
- 显式 `/msg <编号>` 发送一并收紧为仅工作中；回显/撤回不受限。
- 单独 `/msg` 回「用法示例 + 当前工作中 agent 列表（带编号）」。
- 状态窗口与状态栏菜单：仅工作中的 agent 显示「发送消息」。

## 追加需求 D：插话「已阅读」回执（排队消息被消费后回推来源渠道）

**动机**：`/msg` 排队后（回执「已排队 N 条」）目前对方无从得知消息是否被 agent 真正看到。补一条**异步回执**：
排队消息在 agent 下一次工具调用被消费时，回推来源渠道「✅ 你发给 [编号] 的插话已被 Agent 阅读」。

**消费点分析**：`InterjectStore::append` 返回 0＝恰有 hook 在等（即时送达，已回「已送达」）；返回 N>0＝排队。
排队消息的「被消费」唯一发生在 `poll()==Message`（`daemon/mod.rs` AgentEvent `interject_poll` 分支）——
即时送达那条不在此路径。故回执**只覆盖排队→稍后被消费**这一情形。

**数据模型**（`agents/interject.rs`）：
- `Entry` 增 `receipt_channels: Vec<String>`（去重的待回执来源渠道）。不变式：仅当 `entries` 非空时可非空。
- `append(session_id, text, receipt_channel: Option<&str>)`：`try_deliver` 后若 `entries` 仍非空（排队）且给了渠道 →
  去重登记；若已被即时送达（`entries` 空）→ 清空 `receipt_channels`（Q1：即时不回执）。
- `poll()` 的 `Message` 分支：`std::mem::take` 取出 `receipt_channels` 随 `Message` 返回，然后清 `entries`。
- `submit`（GUI 覆盖）/`clear`（撤回）/`remove_session`（结束）/`try_deliver`（即时送达）：清 `entries` 处一并清
  `receipt_channels`（Q2：覆盖/撤回/结束均不回执）。
- `PollOutcome::Message` 由 `Message(String)` 改为 `Message { text, receipt_channels }`。
- 持久化：`Persisted` 增 `receipt_channels: HashMap<sid, Vec<渠道>>`，`#[serde(default)]` 向后兼容
  （不动老 `sessions: HashMap<sid, Vec<String>>` 格式，避免升级丢队列）。

**daemon**：
- `deliver_msg(state, channel_id, session_id, content, lang)` 新增 `channel_id`，内部 `append(.., Some(channel_id))`。
  三条发送路径（显式编号 / 无编号直发 / 单选卡「发送」`msg_pick_deliver`）都带上来源渠道。
- 消费点（`PollOutcome::Message { text, receipt_channels }`）：写回裁决 + persist + 广播后，若 `receipt_channels`
  非空 → `tokio::spawn` 一个任务，按当前快照算 `seq`、`AppConfig::load()`、逐渠道 `reply_channel_text`
  推「autoChannel.msgReadReceipt」（`{id}`＝seq）。热路径仅在有待回执渠道时才 spawn（罕见），零额外开销。

**用户定案（AskHuman）**：Q1 仅排队→消费才回执；Q2 未消费（结束）不回执；Q3 来源渠道持久化；
Q4 文案用「**阅读**」而非「接收/接受」，新推一条、不带内容摘要。

## 追加需求 E：单选卡显示 agent「已运行时长」（便于区分）

**需求**：单选卡列 agent 时，每行显示该 agent 的运行时长，便于区分是哪个 agent。

**用户定案（AskHuman）**：
- 范围：**所有** agent 单选卡（`/watch` `/status` `/unwatch` `/msg` 一致）。
- 位置：选项**主行末尾**追加「· 已运行 X」，排在「· 关注中」徽标之后。
- 口径：复用 watch 卡 `fmt_duration`（据快照 `startedAt` 起算）；单选卡是一次性快照卡，时长是发卡那刻的值（不走字）。

**实现**：`SelectOption` 增 `elapsed: Option<String>`；`agent_options/msg_options/agent_option_by_session/option_from_record`
加 `now` 参数；四渲染器主行 badge 后追加 elapsed；daemon 四处发卡传 `now_secs()`。

**反馈意见（用户，实现后）**：空闲 agent 显示「已运行」易误导 → **改为仅「工作中」显示时长，空闲/其它状态不显示**。
（`option_from_record` 内按 `dot==Working` 门控 elapsed；`<60 秒仍显示 X 秒` 仅对工作中生效。）
