# 实现计划：/watch 实时关注（P1 飞书）

> 设计见 `docs/specs/im-watch.md`。本文件为落地顺序与触点清单。

## P1-1 纯逻辑层

- [x] `paths.rs`：`watch_file()` → `~/.askhuman/state/watch.json`。
- [x] `src-tauri/src/watch.rs`（新模块，全部可单测）：
  - `PersistedWatch { channel, session_id, message_id, created_at }` + `load()/save()`（原子写）。
  - `WatchPhase { Working, Idle, Waiting, Ended }`；`FinalKind { Ended, Cancelled, Replaced }`。
  - `WatchFrame`：由注册表快照记录（`Value`）+ waiting 标志构建（`build_frame`），字段
    seq/kind_label/title/project/phase/text/tool_line/at。
  - `signature()`：状态+标题+文字+工具+时刻+等待 的稳定串。
  - `card_view(frame, lang) -> feishu::card::WatchCardView`（本地化文案在此完成）。
  - `fmt_local_time(epoch)`：今日 `HH:MM:SS`、跨日 `MM-DD HH:MM`（unix `localtime_r`，非 unix UTC）。
- [x] `autochannel.rs`：
  - `Command::Watch(Option<u64>)` / `Command::Unwatch(WatchSel)`（`One(u64)|All|Auto`）+ classify
    （`/watch` `/关注` `/unwatch` `/取消关注`，`all|全部`）。
  - 抽出 `activity_parts(rec) -> ActivityParts{text,tool,at}`（transcript 尾部 × 实时工具融合），
    `status_detail_text` 与 watch 共用；`render_tool` 提为 `pub(crate)`。
  - `help_text` 增加 `watch: bool` 参数（仅飞书列 watch 命令）；watch 列表/回执文案组装函数。
- [x] `i18n.rs`：`watch.*` + `autoChannel.helpCmdWatch` 词条（zh/en）。

## P1-2 飞书卡片

- [x] `feishu/card.rs`：
  - `WatchCardView`（纯字符串视图：header/state_line/title_line/activity_heading/text/tool_line/
    updated_line/buttons）+ `WatchButtons { Active | Final(label) }`。
  - `build_watch_card(&WatchCardView) -> Value`：样式化头部行（`eye_outlined` 蓝）+ hr + markdown
    状态/标题 + markdown 活动 + hr + 灰色小字更新时刻 + column_set 按钮行
    （behaviors callback `value={watch:"unwatch"|"refresh"}`）。
  - `parse_watch_action(event) -> Option<(message_id, WatchAction)>`。
  - 单测：布局、按钮 callback 值、终态禁用、action 解析。

## P1-3 daemon 引擎与命令

- [x] `ServerState` 增 `watch: WatchState { subs: Mutex<Vec<WatchEntry>>, notify: Notify,
      route: Mutex<Option<WatchRouteHandle>> }`（句柄绑定 Router Weak + 已注册 mid 集合，
      任一变化整体重建路由任务）；`WatchEntry { session_id, message_id, seq, created_at,
      last_sig, last_edit_ms, fails, working }`。
- [x] `serve()`：启动时 `watch::load()` 恢复（按 session_id 重解析 seq）+ `spawn_watch_engine`。
- [x] 引擎循环：Notify / 自适应 tick（工作中 2s、空闲 10s）→ `watch_tick`：
  快照 + 在途 ask 集合 → 每订阅算帧 → 签名变化才 `patch_card`；ended → 定格 + 退订 + 持久化；
  连续 5 次失败退订；tick 内幂等 `ensure_watch_routes`。
- [x] `ensure_watch_routes`：飞书 Router 上 `register()` 一条 RoutedFs、`set_active` 所有卡 mid，
  任务循环处理 unwatch/refresh 回调（ack 同步回新卡）；订阅变化 / Router 重建时重启。
- [x] `handle_inbound`：Watch/Unwatch 分派（非飞书回不支持提示）；发卡（`FeishuClient::send_card`）
  / 换新卡 / 列表 / 取消（定格 + 确认文本）。
- [x] Notify 触发点：`AgentEvent`、`handle_submit`（提问创建）、请求完结。
- [x] 闲退守卫：`watch.subs` 非空阻止空闲退出。
- [x] `help_text` 调用点补 `watch` 参数（飞书 true，其余 false）。

## P1-3b 验收反馈批次（跟底重发等）

- [x] `/watch` 无参：提示行在前 + `/status` 同款列表 +「正在关注」段。
- [x] 签名剔除活动时刻 `at`（内容不变不编辑，`signature_ignores_activity_timestamp` 单测）。
- [x] 跟底重发（spec「跟底重发」节）：`WatchState.disturb_ms` 淹没水位线（入站 / 文本回执 /
  提问卡 / 反激活提示 / 补推标记；watch 卡自身不标记）+ `WatchEntry.{sent_at_ms, last_move_ms}`；
  `watch_tick` 内容变化且已淹没且无在途提问且过 30s 节流 → `send_card` 新卡 + 旧卡定格
  `FinalKind::Moved`（「已移至最新卡片 ⬇」）+ 换绑 mid（持久化 + 路由重建）；ended 帧同路径。
- [x] 答复完结（飞书参与）→ 清零全部订阅 `last_move_ms`，下一次内容变化立即跟底。

## P1-4 验证

- [x] 单测（watch.rs / card.rs / autochannel classify+help）。
- [x] `cargo test` 全绿 + `./scripts/install.sh`。
- [ ] 真机（用户验收）：飞书 `/watch`、卡片静默刷新、按钮取消/刷新、agent 结束定格、
      daemon 重启恢复、`/unwatch all`、跟底重发（淹没→新卡接续 / 提问期间抑制 / 答完立即）。

## P2（后续批次，未开始）

- Telegram / Slack / 钉钉（专用模板）各自的 live card 后端。
