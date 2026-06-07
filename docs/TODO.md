# 待办 / 已知问题

记录暂不处理但需跟踪的问题与后续增强。

## 已知问题（钉钉渠道）

### 1. 同一 client-id 同一时刻仅允许一条 Stream（多开相互干扰）

> ⚠️ **由 daemon 架构修复中**：本问题正由 `docs/specs/daemon-architecture.md`（Phase 2：IM 渠道迁入 Daemon + 长连接单实例复用）根治。**待 daemon 架构需求全部开发完成后，删除本条目。**

- **根因**：钉钉官方限制——同一个 client-id 同一时间只允许启动一条 Stream 服务，多开会相互干扰（见官方排查清单）。
- **现状**：当前每次 `AskHuman` 进程各自开一条 Stream（同一 client-id）。正常「一次只问一题」无碍，但**连续快速 / 并发提问时**多条 Stream 会抢消息，可能把用户回复投递到错误的连接。
- **候选修复**：
  1. 文件锁串行化——同一时刻只允许一个进程持有 Stream，退出时发 Close 帧干净断连再释放锁（轻量、无常驻进程；每次提问需重新建连，并发会排队）。
  2. 常驻 daemon——后台进程独占持有 Stream，各 `AskHuman` 经本地 socket 注册等待、由其转发用户消息（真复用 / 连接常热 / 支持并发，但需管理 daemon 生命周期 + IPC，复杂度高）。← **采用此方案（daemon 架构）。**
- **状态**：修复中（daemon 架构 Phase 2）；完成后删除本条目。

## 已修复

### 钉钉卡片「提交」误报『请求失败』toast（实际已成功）✅

> 已修复并真机验证（点提交不再弹『请求失败』、卡片正常置灰、答案正确送达）。

- **根因**：daemon 架构里钉钉长连接由 `dingtalk/router.rs` 的 Reader 独占，Reader 收到卡片回调后**对所有回调一律回空包**。但钉钉**互动卡片「提交」按钮**要求那条 3 秒内的同步回包必须是**非空的成功/更新回包**，否则客户端判定提交失败弹红条（答案其实已送达）。
- **实现方案**（ACK 由「真正接受提交的会话」产出 = 确认而非预测，且读循环不被慢活拖住）：
  1. `dingtalk/card.rs`：新增纯函数 `is_submit(data)`、`submit_ack_success()`（置灰点击者私有 `submitted=true` 的成功回包）。
  2. `dingtalk/router.rs`：Reader 判 `is_submit` —— 非提交回调（选项切换）直接空 ACK、不转发；提交回调转发给对应会话并带 `oneshot` 回执，**带超时(2.5s)等会话裁决**后回包；孤儿/超时回空包（诚实地不显示成功）。
  3. `channels/dingding.rs`：会话认出本卡片提交即**立刻**经 oneshot 回 `submit_ack_success()`（不在 3 秒关键路径上等任何慢活），随后再经 OpenAPI 写公有终态文案；并把作答期间的图片/文件改为**并发下载**（spawn），保证提交一到就能被立刻处理。
- **影响范围与超时取舍**：Reader 等待裁决期间只暂停「当下并发的钉钉」（不影响飞书/Popup/Telegram，各自独立连接），且因会话即时回包＋下载并发化，等待几乎为毫秒级、极少触顶。
### 飞书卡片「提交」按钮置灰有可见闪烁（Loading→弹回 Submit→才变已提交）✅（已大幅改善）

> 同源问题：飞书 Reader 也是收到回调即空 ACK、置灰靠之后的 OpenAPI `patch_card`，导致按钮先弹回再异步变终态。

- **实现**（与钉钉同构，复用飞书已有但未用的 `respond_card` 同步回包）：
  1. `feishu/router.rs`：卡片回调改为**带 oneshot 回执转发给会话**；超时(2.5s)等会话裁决——`Some(body)` → `respond_card` 同步更新卡片、`None`/孤儿/超时 → 空 ACK。
  2. `channels/feishu.rs`：会话认出本卡片提交即**立刻**经 oneshot 回**终态卡片**（`card::callback_update_card` 包装 `build_finalized_card`），按钮 Loading 直接变终态；并把附件下载改为并发。
  3. 去掉了提交路径上的 `patch_card` 兜底（那次二次渲染是残留快速回弹的来源）；被抢答/断连路径仍用 `patch_card`（无回调可同步回包）。
- **现状**：置灰快了很多、不再二次渲染；**仍有一下极快的回弹**——经排查为**飞书客户端自身渲染行为**（收到回调先复位按钮再套用新卡片），非本端可控，保持现状。

### Telegram
- 用 `answerCallbackQuery`，机制不同，**预计无此类问题**；待人工回归确认。

## daemon 架构：待补充的人工实测

> 已通过的真机实测（install 后经新 daemon→GUI Helper 链路）：① 单题弹窗作答（退出 0）；② **并发两请求弹窗不串台**（A→A、B→B）；③ 取消返回 `[Status]` 再问指引（退出 0）；④ `daemon status` 显示 running 且 `im conns: dingtalk, feishu, telegram`（三连接常热单实例）。下列为尚未逐项跑过、建议后续补做的人工测试：

- [ ] **真实 IM 并发（真 TODO#1）**：同时发起两个请求 → 分别在钉钉 / 飞书 / Telegram 的卡片上作答，验证：(a) 回复不串台（按 `outTrackId`/`open_message_id`/callback `message_id` 路由到正确请求）；(b) 自由文字归属正确（Telegram 归「最新活动卡片」、钉钉聊天按 `senderStaffId`、飞书按 `open_id`）；(c) 同一 client_id 仅一条长连接、无多开互抢。
- [ ] **飞书 / Telegram 提交回包**：确认它们点提交**不会**出现钉钉那样的『请求失败』误报（预计不会，模型不同）；若有，按钉钉同款方案修。
- [ ] **被抢答 / 跨渠道抢答**：一个请求同时挂多渠道（弹窗 + IM），在某一渠道作答后，其余渠道卡片应即时置灰为「已在 X 回答」（走 OpenAPI `updateCard`/`patchCard`）。
- [ ] **Phase 3 实时配置（验收 #7）**：弹窗开着时修改 `config.json` 的主题 / 语言（或在设置窗口改并保存）→ 验证打开中的弹窗**实时切换**主题/语言（daemon `config_watch` → `ConfigChanged` → 前端 `settings-updated`）。
- [ ] **Phase 3 凭据热重载（惰性失效）**：修改某渠道凭据 / 禁用某渠道 → 观察 `daemon.log` 出现 `config reloaded`；下一个请求按新配置重连（旧缓存 Router 被丢弃），进行中的请求保留其原连接直到结束。
- [ ] **临时目录清理（A10）**：确认 `temp/askhuman/<id>/` 中超过 24h 未改动的目录会在 daemon 启动时 / 每小时被清理，且不会误删刚产出的图片。
- [ ] **生命周期**：空闲超时自动退出；`daemon stop/restart` 正常；二进制指纹换新（重装后旧 daemon 自动让位、新 daemon 接管）。
- [ ] **自动识别 userId/open_id（Q6）**：设置窗口点「自动识别」→ 经 daemon `Detect`：若已有同 app 长连接则复用观察（零冲突），否则 daemon 临时开连；非 Unix 走进程内回退。

## 后续增强 / 性能优化

### A. 钉钉卡片「变灰」延迟（daemon 架构引入）

- **背景**：daemon 架构下钉钉长连接由 Router 独占共享，卡片回调由 Router 即时空 ACK（满足 3 秒约束），卡片置灰（「已提交」/「已在 X 回答」）改走 OpenAPI `updateCard`（见 `docs/specs/daemon-architecture.md` §11）。
- **影响**：相比单进程时代「stream 同步回包即时变灰」，现在变灰是一次独立 HTTPS 调用，慢约 100–300ms（仅视觉延迟，功能一致）。
- **可选优化**：如需即时变灰，可让会话经 oneshot 把回包回传给 Router 的 Reader，由其在 3 秒内用长连接写回（代价：Reader↔会话耦合、Reader 读循环需短暂等待会话算回包）。当前判断收益有限，暂不做。
