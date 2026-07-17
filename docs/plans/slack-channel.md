# 开发计划：新增「Slack」通信渠道（Channel）

> 关联需求：`docs/specs/slack-channel.md`
> 关联既有：`docs/plans/feishu-channel.md` / `docs/plans/dingtalk-channel.md`（飞书 / 钉钉渠道；Slack 按飞书形态对齐，复用 `run_conversation` 公共驱动）
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
配置(设置页) ──► AppConfig.channels.slack { enabled, botToken(xoxb), appToken(xapp), userId }
                                   │
AskHuman "..." -q ... -o ...       ▼
   └─ run_ask 决策渠道：弹窗(若GUI) + 全部 active 会话型渠道(telegram/dingding/feishu/slack)
        └─ 各会话型渠道 = 复用「公共驱动 run_conversation」+ 各自「MessagingChannel 实现」
             ├─ Telegram：长轮询
             ├─ 钉钉：DingTalk Stream(JSON 帧) + OpenAPI
             ├─ 飞书：Feishu 长连接(protobuf 帧) + OpenAPI
             └─ Slack：Socket Mode(JSON 帧) 收 + Web API 发
                  发：chat.postMessage(text / Block Kit 互动卡片) · chat.update(收尾置终态)
                      conversations.open(解析 DM 频道) · files.getUploadURLExternal +
                      files.completeUploadExternal(上传 -f 文件)
                  收：Socket Mode 两类业务帧 —— events_api(message：文字忽略/图片/文件/识别码)
                       + interactive(block_actions：点提交) ；files[].url_private_download 下载
        └─ Coordinator 抢答：首个终态生效 → render_result → 退出
```

核心三件事：①**新增 Slack 客户端层 `slack/`**（Socket Mode JSON 收 + Web API 发 + Block Kit 卡片 + mrkdwn）；②**新增会话渠道 `channels/slack.rs`**（`SlackChannel` + `SlackSession`），复用 `MessagingChannel` + `run_conversation`；③**daemon / 单进程 / headless 三路集成**（与飞书同构）。

---

## 1. 公共抽象复用（无新增抽象）

`channels/conversation.rs` 的 `MessagingChannel`（`id/open/send_message_prompt/ask_question/close`）与 `run_conversation`（单/多题编排、抢答返回 `None`、完成投递）**保持不变**。Slack 仅：

- 新增传输实现 `SlackSession`（实现 `MessagingChannel`，持 client + Router 事件源句柄 + 跨题状态）。
- 新增薄外层 `SlackChannel`（实现 `Channel`：`start` → spawn → `open` → `run_conversation`；`interrupt` → `Preemption::interrupt`），与 `FeishuChannel` 同构（含 `Own`/`Shared(Arc<SlackRouter>)` 两种 transport）。

> `QuestionCtx`（header/text/options/is_markdown/index/total/lang）与现状一致，Slack 直接消费。

---

## 2. Slack 客户端层 `slack/`（Web API + Socket Mode + Block Kit）

新增模块目录 `src-tauri/src/slack/`，模块边界与飞书 `feishu/` 对齐。

### 2.1 `slack/mod.rs`：错误类型 + 模块声明
- `SlackError`（`EmptyConfig(field)` / `Api(msg)` / `Network(msg)` / `BadResponse`），英文 `Display` + `localized(lang)`（校验类本地化、技术细节英文），与 `FeishuError` 同构。
- 子模块：`client` / `ws` / `blockkit` / `markdown` / `router`。
- 备注：无 token 刷新；Bot Token 走 Web API、App Token 走 Socket Mode 建连。

### 2.2 `slack/client.rs`：`SlackClient`（reqwest，Web API）
- `new(config) -> Result<Self, SlackError>`：校验非空（botToken/appToken/userId）；构造 `reqwest::Client`；持两个 token + userId；DM 频道 id 缓存（`Mutex<Option<String>>` 或在 Session 内缓存均可）。
- 统一调用助手 `call(method, body) -> Value`：`POST https://slack.com/api/{method}`，header `Authorization: Bearer <bot_token>`，按 `ok==true` 判定成功，失败取 `error`。
- **鉴权 / 探测**：`auth_test()`（`auth.test`，校验 Bot Token 并取 bot user id）。
- **解析 DM 频道**：`open_dm() -> channel_id`：`conversations.open {users: userId}` → `data.channel.id`，结果缓存（首次解析后复用）。
- **发送消息**：`post_message(blocks, text_fallback) -> ts`：`chat.postMessage {channel: dm, text, blocks}`；返回 `data.ts`。便捷：`post_text(mrkdwn)`（仅 text）。
- **更新消息（收尾用）**：`update_message(ts, blocks, text_fallback)`：`chat.update {channel, ts, blocks, text}`。被抢答/提交/取消时置静态终态。
- **上传媒体（AI→人，新版三步）**：`upload_file(path, name, is_image) -> ()`：
  1. `files.getUploadURLExternal {filename, length}` → `{upload_url, file_id}`。
  2. PUT/POST 文件字节到 `upload_url`。
  3. `files.completeUploadExternal {files:[{id:file_id, title:name}], channel_id: dm}` → 分享进 DM。
- **下载用户文件（人→AI）**：`download_file_to(url_private_download, ext) -> 本地路径`：GET 该 URL，header `Authorization: Bearer <bot_token>` → 字节落地临时目录（`askhuman/<...>` 复用现有 temp 规则），按 `mimetype`/`filetype` 修正扩展名。
- **App Token 探测**（供测试连接）：`open_connection_url(app_token) -> wss_url`：`POST apps.connections.open`，header `Authorization: Bearer <app_token>`（**必须放 header**），取 `url`。供测试连接校验与 ws 建连复用。

### 2.3 `slack/ws.rs`：Socket Mode（JSON 帧，飞书 `ws.rs` 的 Slack 版）

职责与 `feishu/ws.rs` 对齐，但**更简单**（JSON 帧 + 收帧即 ack，无 protobuf、无延迟回包）。

- **建连**：`connect(http, app_token, bot_token) -> Self`：调 `open_connection_url` 取 wss → `tokio-tungstenite` 连接（rustls）。
- **帧循环 `recv() -> Option<WsEvent>`**：读 `Message::Text`，`serde_json` 解析帧，按 `type`：
  - `hello`：建连确认，忽略（无 `envelope_id`）。
  - `disconnect`：服务端要求重连（reason `warning`/`refresh_requested`/`too_many_connections`）→ 重连（重新 `open_connection_url` + 连接）。
  - `events_api`：**先回 ack**（`{"envelope_id": id}`）→ 取 `payload.event`，若 `type=="message"`（且非 bot 自身消息：无 `bot_id`、`subtype` 不在忽略集如 `bot_message`/`message_changed`/`message_deleted`）→ `WsEvent::Message(event)`；否则忽略。
  - `interactive`：**先回 ack**（`{"envelope_id": id}`，空 payload）→ 若 `payload.type=="block_actions"` → `WsEvent::Interactive(payload)`；其它（如 `view_*`）忽略。
  - `slash_commands` 等：ack 后忽略。
  - WS 协议 `Ping` → 回 `Pong`；`Close`/`Err`/`None` → 重连（按重试上限），耗尽 `recv()` 返回 `None`。
- **ack**：`ack(envelope_id)` 发 `Message::Text("{\"envelope_id\":\"...\"}")`。**与卡片更新解耦**：ws 层收到含 `envelope_id` 的业务帧立即 ack（满足 3 秒），卡片更新由会话层经 `chat.update` 另行完成。
- 对上层暴露：`connect(...)`、`recv()`。**无 `respond_card`/oneshot**（与飞书的关键简化点）。
- 诊断日志：仿 `feishu::ws::debug_log`，环境变量 `ASKHUMAN_SLACK_DEBUG=1` 时写 `~/.askhuman/slack-debug.log`（默认关闭）。

> `WsEvent { Message(Value), Interactive(Value) }`：两类皆已 ack。

### 2.4 `slack/blockkit.rs`：Block Kit 卡片组装 + 回调解析（纯函数，可单测）
- `build_question_card(header, text, options, is_markdown, placeholder, submit_label) -> Vec<Block>(JSON)`：
  - section 块：标题（加粗）+ 正文（`is_markdown` ? mrkdwn(经 §2.5) : 纯文本）。section 文本 ≤3000 字符（超长截断/拆块）。
  - **复选框**：有选项时一个或多个 `input` 块，内 `checkboxes`（`action_id="options"`，每块 ≤10 项，`optional:true`，每项 `{text, value:"opt_{i}"}`，下标全局连续）；不设 `dispatch_action`。
  - **输入框**：一个 `input` 块，内 `plain_text_input`（`action_id="user_input"`, `multiline:true`, `optional:true`, placeholder）。
  - **提交按钮**：一个 `actions` 块，内 `button`（`action_id="submit"`, text=submit_label, value `"submit"`）。
  - 给各 `input` 块固定 `block_id`（如 `opts_{k}` / `userinput`）便于回调取 `state.values`。
- `parse_submit(payload, options) -> Option<CardSubmit>`：
  - 校验 `payload.type=="block_actions"` 且 `actions[].action_id=="submit"`。
  - 取 `payload.container.message_ts`（卡片 ts）、`payload.user.id`（用户）、`payload.channel.id`（频道）。
  - 遍历 `payload.state.values[block_id][action_id]`：`options` 复选框 → `selected_options[].value`，按 `opt_{i}` 还原选项文本数组；`user_input` 文本（空→None）。
  - `CardSubmit { user_id, message_ts, channel_id, selected_options, user_input }`；缺字段不 panic，返回 `None`。
- `build_finalized_card(Finalized{header,text,is_markdown,options,selected,user_input,status}) -> Vec<Block>`：静态终态——标题 + 正文 + 已选项打勾回显（`✓ 选项`）+ 补充文字回显（如有）+ 状态行；**无任何交互控件**。被抢答（本端未作答）时 `selected`/`user_input` 传空。
- 纯函数单测：卡片组装（含/不含选项、md/非 md、>10 选项拆块）、`parse_submit`（勾选还原、空输入、缺字段、多复选框块合并）。

### 2.5 `slack/markdown.rs`：标准 Markdown → Slack mrkdwn（仿 `telegram/markdown.rs`）
- 转换：`**粗**`/`__粗__` → `*粗*`；`*斜*`/`_斜_` → `_斜_`；`~~删~~` → `~删~`；`` `码` `` 保留；```` ``` 块 ```` 保留；`> 引` 保留；`[文字](url)` → `<url|文字>`；列表 → `• ` 项目符号；表格 → 等宽代码块；标题 `#` → 加粗行。
- 转义：仅转义 Slack 特殊字符 `&`、`<`、`>`（其余原样），与 telegram「标签天然配对不回退」思路一致。
- 纯函数单测覆盖各语法。

---

## 3. Slack 会话渠道 `channels/slack.rs`

### 3.1 外层 `SlackChannel`（实现 `Channel`，与 `FeishuChannel` 同构）
- 持 `SlackChannelConfig + Arc<Preemption> + SlTransport{Own|Shared(Arc<SlackRouter>)}`；`id()="slack"`。
- `start`：spawn task → 取事件源句柄（`Own`：`SlackRouter::connect` 现连；`Shared`：复用）→ `SlackSession::new(config, events)` → `open()`（失败 i18n 警告并跳过）→ `run_conversation`。
- `interrupt(reason)` → `preempt.interrupt(reason)`。
- `new(config)`（单进程）/ `shared(config, router)`（daemon）两个构造，与飞书一致。

### 3.2 `SlackSession`（实现 `MessagingChannel`）
- 持有：`SlackClient`（发送/更新/上传/下载）、`RoutedSlack`（Router 事件源句柄，长连接由 Router 独占）、`dm_channel` 缓存。
- `open()`：构造 client（校验 botToken/appToken/userId 非空）+ `auth.test`（隐式或在 connect 时）+ `open_dm()` 缓存 DM 频道。失败 → `Err`（外层警告跳过）。
- `send_message_prompt(message, is_markdown, source, lang)`：
  - 头部「`「Message from {source}」`」+ 文本：`post_text`（mrkdwn）。
  - `-f` 文件：`upload_file`（图片/文件均走新版三步）；失败 → i18n 警告 + 发一条含文件名的失败提示文本（不中断）。**不做钉钉式 inline/docx**。
  - 适当 settle 延迟（沿用飞书 `MESSAGE_SETTLE_DELAY`，保证「先 Message 后题目」视觉顺序）。
- `ask_question(ctx, preempt)`：**卡片流程**
  1. `blocks = blockkit::build_question_card(ctx.header|兜底, ctx.text, ctx.options, ctx.is_markdown, placeholder, submit)`；`ts = client.post_message(blocks, fallback_text)`。
     - 发送失败 → i18n 警告 → **回退 B 方案** `ask_question_text(...)`（§3.3）。
  2. `events.set_active(ts, user_id)`（登记卡片 ts 精确路由 + 认领该 user 的 DM 消息）。
  3. 事件循环（每 `POLL_INTERVAL`≈1s 检查 `preempt.is_cancelled()`，`recv()` 加 timeout 以便分片检查抢答；图片/文件**并发下载**，仿飞书）：
     - `WsEvent::Interactive(payload)`：`parse_submit` 命中且 `message_ts==ts`、`user_id==配置 userId` →
       - 收尾并发下载 → 组装 `QuestionAnswer { selected_options, user_input, images, files }`；
       - `client.update_message(ts, build_finalized_card(..,「已提交」))` 置静态终态；
       - `events.clear_active(ts, user_id)`；返回 `Some(answer)`。
       - 否则（非本卡片/非提交/解析失败）→ 忽略，继续等待（帧已由 Router ack）。
     - `WsEvent::Message(event)`：`event.user==配置 userId` →
       - `files[]` 里图片 → 下载累积进 `images`（转 base64 `ImageAttachment`）。
       - `files[]` 里其它文件 → 下载落地累积进 `files`。
       - 纯文字 / 其它 → **忽略**（请用卡片输入框）。
     - `recv()` 返回 None（断连重连耗尽）→ 跳出收尾。
  4. `preempt.is_cancelled()` / 断连收尾 → best-effort `client.update_message(ts, build_finalized_card(.., 状态))`（状态按 `preempt.reason()`：`AnsweredBy`→「已在X回答」/`Cancelled(src)`→「已被X取消」/否则「已取消」；本端未作答故不回显勾选/文字）→ 返回 `None`。
- `close()`：丢弃事件源句柄 → 从 Router 注销路由（`self.events = None`）。
- 累积状态（images/files）为**单题局部**变量，随每次 `ask_question` 重置（多题互不串味）。

### 3.3 回退 B 方案 `ask_question_text`（卡片失败时）
- 与飞书同形：发「头部 + 正文 + 编号选项 + 作答提示」纯文本（mrkdwn），用户回一条 DM 完成该题。
- 复用解析：`parse_reply`（纯编号→映射选项，否则自由文本）、`message_to_answer`（text/image/file → 回答）。
- 这些纯文本辅助函数与飞书 `channels/feishu.rs` 内同名逻辑高度一致，在 Slack 文件内各自实现（不强求跨渠道再抽公共函数）。

### 3.4 事件字段解析（关键）
- `message`（DM 事件）：`event.user`（发送者）、`event.channel`（DM 频道）、`event.ts`、`event.text`、`event.files[]`（`{id, url_private_download, name, mimetype, filetype}`）；忽略 `bot_id`/`subtype∈{bot_message,message_changed,message_deleted,...}`。
- `block_actions`（交互）：见 §2.4 `parse_submit`（`container.message_ts` / `user.id` / `channel.id` / `state.values` / `actions[]`）。

---

## 4. Slack 长连接 Router `slack/router.rs`

进程内独占一条 Socket Mode 连接，把事件按 **`message_ts`（交互回调）/ `user_id`（DM 消息）** 分发到对应会话。设计与 `feishu/router.rs` 同构，但**无 oneshot ack**（ws 层已收帧即 ack）。

- `SlackRouter`：持 `app_token`（作为「自动识别」复用现有连接的匹配键）、`Routes{ cards: HashMap<ts,route_id>, loose: HashMap<user_id,route_id>, sinks: HashMap<route_id,Sender<SlackInbound>>, observers: Vec<Sender<Value>> }`、`alive: AtomicBool`、reader 任务句柄（`Drop` 中 abort 关连接）。
- `connect(config) -> Result<Arc<Self>, String>`：建 `SlackClient`（取 app_token/bot_token）→ `FeishuWs` 对应的 `SlackWs::connect` → spawn `reader_task`。失败返回英文错误。
- `register() -> RoutedSlack`：登记一条路由（`set_active(message_ts, user_id)` / `clear_active` / `recv`）。
- `observe_message() -> UnboundedReceiver<Value>`：原始 message 事件观察者（供「自动识别 userId」）。
- `app_token()` / `is_alive()`。
- `SlackInbound { Interactive(Value), Message(Value) }`（皆已 ack）。
- reader_task：`ws.recv()` 循环 → `WsEvent::Interactive` 按 `container.message_ts` 路由到 `cards`；`WsEvent::Message` 先广播 observers，再按 `event.user` 路由到 `loose`。连接断开 → `alive=false` + 清空 sinks（各会话 `recv()` 得 `None`）。

---

## 5. 配置、命令与设置页 UI

### 5.1 配置 `config.rs` / 密钥 `secrets.rs` / 类型 `types.ts`
- 新增 `SlackChannelConfig { enabled:bool, bot_token:String, app_token:String, user_id:String }`（serde camelCase + `#[serde(default)]`）。`Default`：`enabled=false`，三项空串。
- `ChannelsConfig` 增 `slack: SlackChannelConfig`。
- `secrets.rs` 增两个 account：`ACCOUNT_SLACK_BOT_TOKEN="channels.slack.botToken"`、`ACCOUNT_SLACK_APP_TOKEN="channels.slack.appToken"`；`config.rs` 的 `SECRET_SPECS` 增两项（分别映射 `bot_token` / `app_token`）。
- `config.rs` 单测补充 Slack 默认值 + 旧 JSON 无 `slack` 仍可加载。
- TS `types.ts` 同步 `SlackChannelConfig` + `ChannelsConfig.slack` + 命令参数类型。

### 5.2 命令 `commands.rs`（+ `app/mod.rs` 注册 + `ipc.ts`）
- `slack_test(args{botToken, appToken, userId}) -> Result<String,String>`：userId/token 空给中文提示（`cmd.fillBotAppToken`/`cmd.fillUserId`）；`auth.test`（校验 Bot Token）+ `open_dm` + `post_text` 测试 DM；再 `apps.connections.open` 校验 App Token；成功返回提示（`cmd.slTestSent`）。secret 走 `fallback_secret`（表单空时回退已存钥匙串值，与飞书一致）。
- `slack_detect_prepare(args{botToken, appToken}) -> Result<String,String>`：前置校验（token 空 → 中文错误）；通过返回随机 4 位识别码。
- `slack_detect_wait(args{appToken, botToken, code}) -> Result<String,String>`：**先经 daemon `client::request_detect`**（复用现有 `Detect` 通道）；daemon 不在则回退本进程直接建 `SlackWs` 等到 `message` 且 `text==code` 的 DM，返回其 `event.user`；~120s 超时报错。复用 `cmd.detectTimeout`/`detectCodeInvalid`/`streamDisconnected`。
- 全部 `async` 命令；在 `app/mod.rs` 的 `invoke_handler` 注册；`ipc.ts` 增对应封装。

### 5.3 IPC `ipc/mod.rs`（复用 `DetectRequest`，不改协议版本）
- `DetectRequest.kind` 增 `"slack"`；**字段复用**：`app_key=appToken`（也是复用现有连接的匹配键）、`app_secret=botToken`、`base_url=""`。无 schema 变更 → `PROTOCOL_VERSION` 不变。
- `StatusInfo.im_connections` 文档注释增 `"slack"`。

### 5.4 设置页 `SettingsView.vue`（「通信渠道」tab）
- 仿飞书卡片新增「Slack」卡片：`enabled` 开关 + 字段 Bot Token(password) / App Token(password) / User ID（旁置「自动识别」按钮，沿用两段式：prepare 取码 → 展示「请私聊机器人发送：XXXX」→ wait 回填）。
- 「测试连接」按钮 → `slack_test`，复用现有 `result ok/err` 展示。
- 复用现有 detect/test 的 loading/错误状态写法（`slackTesting`/`slackDetecting`/`slackDetectCode`/`slackMessage`/`slackError`）。
- 位于其他已支持的外部 IM 渠道之后；页面不保留“更多渠道敬请期待”占位卡。
- `src/i18n/zh.ts` / `en.ts` 设置页键：`settings.channels.slackTitle` / `botToken` / `appToken` / `userId`（含占位说明）等；detect/test 复用现有 `detecting`/`autoDetect`/`detectHint`/`detected`/`testing`/`testConnection`。

---

## 6. 运行编排集成（`app/mod.rs` + `daemon/mod.rs` + `coordinator.rs`）

### 6.1 单进程 / headless（`app/mod.rs`）
- 新增 `is_slack_active(config)`：`enabled` 且 `!user_id.trim().is_empty()` 且 `SlackClient::new(&config.channels.slack).is_ok()`（两 token 非空）。
- `has_active_messaging` 增 `|| is_slack_active`。
- `active_messaging_channels`：active 时 push `SlackChannel::new(config.channels.slack.clone())`。
- `run_headless`：`messaging_count` 增 `is_slack_active as usize`；新增 Slack 分支（与飞书同构：`SlackRouter::connect` → `register` → `SlackSession::open` → `run_conversation`）。

### 6.2 daemon（`daemon/mod.rs`，与飞书同构）
- `ServerState` 增 `sl_router: tokio::sync::Mutex<Option<Arc<SlackRouter>>>`。
- `ensure_sl_router(state, cfg)`：惰性建连 / 死亡重连（仿 `ensure_fs_router`）。
- `attach_im_channels`：`is_slack_active` 时 `ensure_sl_router` → `SlackChannel::shared` → `register` + `start`；失败经 `Warn` 给 CLI stderr（`channel.slConfigInvalidSkip`）。
- `active_im_connections`：存活时 push `"slack"`。
- `handle_detect`：`"slack"` → `detect_slack`（观察现有同 `app_token` 的活动 Router，否则临时开连）→ `wait_slack_code`（等 `message` 且 `text==code` → `event.user`，120s 超时）。
- `on_config_changed`/`invalidate_changed_routers`：`sl_changed = !is_slack_active(new) || bot_token/app_token/user_id 变更` → 丢弃缓存 `sl_router`。
- 收尾（serve 退出）：`*state.sl_router.lock().await = None`（Drop 关连接）。

### 6.3 抢答展示名（`coordinator.rs`）
- `display_name`：增 `"slack" => channel.sourceSlack`。

退出码 / 抢答语义不变。

---

## 7. i18n

- Rust `i18n.rs` 新增键（中/英，沿用 `channel.*`/`cmd.*`/`err.*`/`app.*` 命名，仿飞书 `fs*` 用 `sl*` 前缀）：
  - `channel.sourceSlack`（「Slack」/「Slack」）。
  - `channel.slSubmitted` / `slAnsweredVia` / `slCancelled` / `slCancelledBy` / `slTitleFallback` / `slInputPlaceholder` / `slSubmitButton` / `slConfigInvalidSkip` / `slMessageSendFailed` / `slFileSendFailedLog` / `slQuestionSendFailed` / `slCardDeliverFailed` / `slImageDownloadFailed` / `slFileDownloadFailed`（多数借鉴飞书 `fs*` 文案）。
  - `app.slackInvalid`（headless 警告）。
  - `cmd.slTestSent` / `cmd.fillBotAppToken` / `cmd.fillUserId`；`err.slEmptyConfig`。
  - detect 复用现有 `cmd.detectTimeout` / `cmd.detectCodeInvalid` / `cmd.streamDisconnected`。
- 前端 `src/i18n/zh.ts` / `en.ts`：设置页 Slack 键；detect/test 复用现有键。

---

## 8. 依赖（`Cargo.toml`）

- **不新增任何 crate**。复用：`tokio-tungstenite`(rustls,已有)、`futures-util`(已有)、`reqwest`(json/multipart/rustls,已有)、`serde_json`、`base64`、`tokio`、`uuid`、`async-trait`。
- 不引入第三方 Slack SDK（体积/构建/维护/供应链考量，与手写钉钉/飞书风格一致）。

---

## 9. 涉及文件清单

新增：
- `src-tauri/src/slack/{mod.rs, client.rs, ws.rs, blockkit.rs, markdown.rs, router.rs}`。
- `src-tauri/src/channels/slack.rs`：`SlackChannel` + `SlackSession`（+ 回退 `ask_question_text`）。
- `docs/specs/slack-channel.md` / `docs/plans/slack-channel.md`（本两份）。
- `docs/wiki/slack-setup.md` / `docs/wiki/slack-setup.en.md`（前置条件 / scopes / Socket Mode / Interactivity / message.im）。

改动：
- `src-tauri/src/channels/mod.rs`：`pub mod slack;`。
- `src-tauri/src/config.rs`：`SlackChannelConfig` + `ChannelsConfig.slack` + `SECRET_SPECS` 两项 + 单测。
- `src-tauri/src/secrets.rs`：两个 Slack account 常量。
- `src-tauri/src/commands.rs` / `src/lib/ipc.ts`：`slack_test` / `slack_detect_prepare` / `slack_detect_wait`（+ 注册）。
- `src-tauri/src/ipc/mod.rs`：`DetectRequest`/`StatusInfo` 注释（slack 映射，无 schema 变更）。
- `src-tauri/src/app/mod.rs`：`is_slack_active`、`active_messaging_channels`、`has_active_messaging`、headless 分支与计数、命令注册。
- `src-tauri/src/daemon/mod.rs`：`sl_router`、`ensure_sl_router`、`attach_im_channels`、`active_im_connections`、`detect_slack`/`wait_slack_code`、config-watch 失效、收尾。
- `src-tauri/src/app/coordinator.rs`：`display_name` 增 slack。
- `src-tauri/src/i18n.rs` / `src/i18n/zh.ts` / `src/i18n/en.ts`：Slack 文案。
- `src/lib/types.ts`：`SlackChannelConfig` + `ChannelsConfig.slack` + 命令参数类型。
- `src/views/SettingsView.vue`：Slack 配置卡片 + 自动识别/测试。
- `src-tauri/src/prompts.rs` / `README.md` / `README.en.md` / `docs/overview.md`：文档（含前置条件、已知问题）。

---

## 10. 任务顺序

1. **客户端底座**：`slack/mod.rs`（错误类型）+ `slack/client.rs`（call/auth_test/open_dm/post_message/update_message + 上传三步 + 下载 + open_connection_url）。
2. **Socket Mode**：`slack/ws.rs`（建连 / JSON 帧循环 / 收帧即 ack / hello·disconnect·ping / 重连 / `WsEvent`）。
3. **配置/密钥/类型**：`SlackChannelConfig`（Rust + TS）+ `SECRET_SPECS` + secrets 常量 + 默认值/容错单测。
4. **联调收发底座**：`slack_test` 验证发（auth.test + 测试 DM + apps.connections.open）；`slack_detect_prepare/wait` + Socket Mode 验证收（`message` 事件取 `event.user`）。
5. **互动卡片**：`slack/blockkit.rs`（组装 + `parse_submit`）+ `slack/markdown.rs`（mrkdwn）；打通「勾选 + 补充文字 + 提交 + state.values + chat.update 终态」。
6. **Router**：`slack/router.rs`（`SlackRouter` + `RoutedSlack` + observers，路由键 ts/user_id）。
7. **会话渠道**：`channels/slack.rs`（`SlackChannel` + `SlackSession`）串起发 Message / 逐题卡片 / 收图片文件 / 完成；接入 `run_conversation`；卡片失败回退 `ask_question_text`。
8. **编排集成**：`app/mod.rs`（active 判定 + headless + 计数 + 命令注册）；`daemon/mod.rs`（router/attach/detect/status/config-watch）；`coordinator` display_name。
9. **设置页 UI + ipc/types + i18n**。
10. **文档**：`prompts.rs` / `README` / `docs/wiki/slack-setup.*` / `docs/overview.md`。
11. **构建 + 安装实测**：`pnpm build && cargo build --release --features custom-protocol`；`./scripts/install.sh`，用新装 `AskHuman` 端到端验收。

---

## 11. 测试策略

- Rust 单测：
  - `config` Slack 默认值 / 容错（旧 JSON 无 slack）。
  - `blockkit` 纯函数（卡片组装含/不含选项与 md 切换、>10 选项拆块；`parse_submit` 勾选还原、空输入、缺字段、多复选框块合并）。
  - `markdown` 各语法转 mrkdwn round-trip。
  - `client` 纯函数（Web API body 组装、扩展名/mimetype 映射）。
  - `ws` 可纯测部分（帧 `type` 分派判定、ack 报文组装）。
- 手动 / 端到端（需真实 Slack App + Socket Mode + Interactivity + message.im）：
  - 测试连接（Bot + App Token）/ 自动识别 userId。
  - 单题 / 多题 / 带 Message + `-f` 文件（图片、文件可发可见）。
  - 卡片多选勾选 + 补充文字 + 提交完成；提交后卡片静态终态、回显选择与文字。
  - 作答期间发图片/文件被收进答案；DM 纯文字被忽略。
  - 卡片投放失败回退纯文本编号选项。
  - 与弹窗 / Telegram / 钉钉 / 飞书 抢答（Slack 先答 / 被抢答中止并尽力置卡片终态）。
  - headless（无 GUI）单跑 Slack；与其它渠道并行。
  - `daemon status` 计入 slack；改配置经 config-watch 即时失效重连。
  - 弹窗 / Telegram / 钉钉 / 飞书 回归。

---

## 12. 风险与注意

- **Socket Mode 回包结构 / 字段路径**：以真机联调为准固定 `apps.connections.open` 响应、`events_api`/`interactive` 帧字段（`payload.event` / `payload.state.values[block_id][action_id]` / `container.message_ts`），并在 `ws.rs`/`blockkit.rs` 注释锁定。
- **3 秒 ack**：每条含 `envelope_id` 的业务帧收到后立即 ack（与卡片更新解耦），避免重推。
- **消息内 input 块取值**：依赖「点按钮的 block_actions 携带整条消息 state.values」；真机确认后锁定（spec §6 已登记）。
- **集群多开互抢**（spec §6 已知问题）：daemon 单连接根治；单进程回退维持不修，仅记录。
- **复选框 10 项上限 / section 3000 字符**：拆块 / 截断处理，不崩溃。
- **文件上传新流程**：三步缺一不可，`completeUploadExternal` 必带 `channel_id`；以真机固定字段。
- **断线重连**：长等待期间 socket 可能断（含服务端 `disconnect` 主动要求）；重连后继续等本卡片回调/消息，不丢「当前题等待中」状态。
- **被抢答更新卡片**：`chat.update` 用本应用发送的消息（自身可更新）；失败仅日志，不影响主流程。
- **mrkdwn 方言**：与标准 Markdown 不同；先基础转换，异常再补。
- **配置容错**：旧配置无 `slack` 走默认（`#[serde(default)]`）。
- **release 构建**：须 `--features custom-protocol`；不新增 crate。
