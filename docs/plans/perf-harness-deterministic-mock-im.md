# 计划：确定性弹窗性能基线 harness + 全 Channel Mock IM

目标：把弹窗启动性能测试做成**无脑、确定性、可防未来劣化**的一条 canonical 场景；并用**本地 mock IM（覆盖全部 4 个 channel）**确定性地把「IM 挂接」纳入弹窗关键路径的回归监控。需求与度量方法论见 `docs/specs/popup-launch-performance.md`。

## 设计原则（用户确认）

1. **无脑单命令**：`node scripts/perf-popup.mjs` 不带参就跑固定场景；固定基线路径 `docs/perf/baseline.json`——有基线则对比、无则创建；端到端 p90 劣化则退出非零。仅保留 `--update-baseline`（显式刷新基线）。**去掉所有改变「测什么」的参数**（runs/warmup/cold/timeout 等全部内置固定值）。
2. **覆盖要面向未来**：不以当前行为裁剪测试。即使现在 IM 不在问题区，也要把它纳入，未来若有改动让 IM 阻塞弹窗启动即被暴露。
3. **冷 + 热都跑**：同一次执行内跑两组（cold = 每轮前停隔离 daemon；warm = daemon 全程在线），各出一组数，基线含两组。
4. **全 Channel mock**：mock 覆盖全部支持的 channel（钉钉/飞书/Telegram/Slack），让 channel 内部逻辑（建连 + 发卡片）也被走到、有问题能暴露。
5. **故意加延迟**：mock 的「建连」响应**故意加 ~150ms 延迟**；若未来改动让建连阻塞弹窗启动，该延迟会进端到端、在基线对比中报红。
6. **零副作用 + 隔离**：全程在临时 `HOME` 的隔离 daemon 下、连本地 mock，绝不碰用户真实 daemon / 真实 IM 群。

## 隔离与确定性（已验证）

- 所有运行时路径都在 `$HOME/.askhuman` 下（socket/lock/perf.log/agents/config）。harness 用临时 `HOME` 起**独立 daemon**，与真实 daemon 并存互不干扰。
- canonical 配置：harness 在隔离 `HOME` 写一份 `config.json`，**启用全部 4 个 channel**、凭据为占位串、base URL 指向本地 mock，并开 `channels.popup.enabled`。
- **零钥匙串副作用（实现时补）**：OS 钥匙串**不随 `HOME` 隔离**，`AppConfig::load()` 会把 config 里的明文占位密钥**迁移进/覆盖用户真实钥匙串**。故新增测试开关 `ASKHUMAN_NO_KEYCHAIN=1`（`secrets.rs`：等价「钥匙串不可用→明文回退」，零钥匙串读写、跨重载幂等），harness 对所有子进程设置之。
- **屏幕可见守卫（实现时补）**：`fe.painted` 靠真实 WebView 的 `requestAnimationFrame`，macOS 锁屏/息屏/遮挡时会暂停 rAF → 弹窗不上屏、autodismiss 不触发、ask 挂超时。故 harness 启动前与每轮前读 `ioreg -n Root -d1 -r` 的 `CGSSessionScreenIsLocked`，**锁屏即报错不跑**；运行期开 `caffeinate -d` 防息屏；任一轮超时即判数据无效、报错中止。

## 改动一：Rust 侧 base URL 可被环境变量覆盖（仅测试用，默认不变）

部分 channel 的 endpoint 是硬编码，需加 env 覆盖（未设时行为完全不变）：

- **DingTalk**：`client.rs`(`API_BASE`=api.dingtalk.com、`OAPI_BASE`=oapi.dingtalk.com)、`token.rs`、`stream.rs`(gateway/connections/open) → 读 `ASKHUMAN_DINGTALK_API_BASE`（覆盖 api.dingtalk.com，含 token/gateway/卡片）。oapi 若 mock 用不到可暂不覆盖。
- **Slack**：`client.rs`(`API_BASE`=slack.com/api)、`ws.rs`(`CONNECTIONS_OPEN`) → 读 `ASKHUMAN_SLACK_API_BASE`（覆盖 slack.com/api）。
- **Telegram**：已有 `channels.telegram.apiBaseUrl` 配置，harness 经 config 指向 mock，无需 env。
- **Feishu**：已有 `channels.feishu.baseUrl` 配置，token/open 走它；ws 地址由 open 响应返回（mock 回 `ws://localhost`），无需 env。

实现方式：在各 base 常量取用处包一个 `fn api_base() -> String { env::var("ASKHUMAN_*_API_BASE").unwrap_or(默认) }`，集中一处，最小改动。

## 改动二：本地 Mock IM 服务（Node，`scripts/perf-mock-im.mjs`）

单进程，监听一个本地端口，按路径区分 channel。**两条路径都要 mock 且都覆盖到位**：

1. **建连**（`Router::connect()`，当前**被 await、在弹窗关键路径上**）：提供最小握手让 connect 成功且 `is_alive()` 持续为真。
2. **发消息/卡片**（`ch.start()` → 内部 `session.open()` + HTTP 发卡片）：mock 接受该发送请求并返回 ok，**驱动 channel 自己的发卡片代码跑通**（建卡片、序列化、HTTP 调用都真实执行，逻辑有 bug 能暴露）。

> 现状说明（决定延迟放哪）：四家 channel 的 `ch.start()` 都用 `tauri::async_runtime::spawn(...)` 把「发卡片」**detached 异步**执行，**当前不阻塞弹窗 spawn**；只有「建连」是 `await` 的、在关键路径上。因此：
> - **建连**响应故意 sleep ~150ms → 直接进端到端，守当下「建连阻塞弹窗」。
> - **发消息**响应也故意 sleep ~150ms → 当前因异步不进端到端；但**未来若有人把发送改成阻塞/前置到 spawn 前**，这 150ms 就会冒进端到端、在基线对比报红。两处都加 = 当下 + 未来双保险（用户明确要求发送也要能暴露阻塞）。
>
> mock「最小」指**不复刻完整业务协议**（不模拟用户经 IM 回信、不实现全部消息类型）——因弹窗 autodismiss、无需用户回信即可结束。但**建连 + 发送两条路径都真实走通**，不是只 mock 建连。

各 channel 最小实现：

- **Telegram**（HTTP）：`/bot<token>/getMe`→`{ok:true,result:{id,is_bot:true,...}}`；`/getUpdates`→延迟后 `{ok:true,result:[]}`（长轮询空）；`/sendMessage`→`{ok:true,result:{message_id:1,...}}`。
- **Slack**：`POST /api/apps.connections.open`→（延迟后）`{ok:true,url:"ws://host:port/slack-ws"}`；`/api/auth.test`、`/api/chat.postMessage`→`{ok:true,ts:"1"}`；WS 端点接受握手、连上即发 `{"type":"hello"}`、保持打开、回 pong。
- **DingTalk**：`POST /v1.0/oauth2/accessToken`→`{accessToken,expireIn:7200}`；`POST /v1.0/gateway/connections/open`→（延迟后）`{endpoint:"ws://host:port/dd-ws",ticket:"t"}`；卡片相关 api 路径→`{}`/ok；WS 端点接受 `endpoint?ticket=…` 握手、保持打开。
- **Feishu**：token 端点→`{code:0,tenant_access_token,expire:7200}`；gen endpoint（open）→（延迟后）`{code:0,data:{URL:"ws://host:port/fs-ws?...",...}}`；发消息路径→`{code:0,data:{message_id:"m"}}`；WS 端点接受握手、保持打开。

说明：mock 只需让 connect 成功 + 连接保活 + 接受发送；不复刻完整业务协议（弹窗会自动取消，不需要模拟用户回信）。这已能覆盖「建连是否阻塞弹窗」与「发卡片代码是否跑通」。

## 改动三：harness 重构（`scripts/perf-popup.mjs`）

- 启动 mock（`perf-mock-im.mjs`，随机端口），建隔离 `HOME` + 写 canonical `config.json`（4 channel 指向 mock + 占位凭据 + popup.enabled）+ 设 `ASKHUMAN_*_API_BASE` env。
- 跑两组（各 N=20、warmup=2，内置固定）：
  - **cold**：每轮前 `daemon stop --force`（隔离），再发一次 `AskHuman` 提问（autodismiss）。
  - **warm**：先 `daemon start` 起隔离 daemon，连续 N 次。
- 解析隔离 `perf.log`，按 `perf_id` 聚合，输出 cold/warm 两张表（各段中位/p90 + im_attach 段）。
- 基线：`docs/perf/baseline.json` 存 `{cold:{...}, warm:{...}}`；无则创建、有则对比；端到端（spawn→painted）p90 劣化 > 阈值（默认 20%）退出非零。`--update-baseline` 刷新。
- 退出前停 mock + 停隔离 daemon + 删临时 `HOME`。

## 影响文件

- 新增：`scripts/perf-mock-im.mjs`（mock 服务）。
- 改：`scripts/perf-popup.mjs`（无脑化 + 起 mock + canonical config + cold/warm 双跑 + 固定基线路径）。
- 改：`src-tauri/src/{dingtalk/client.rs,dingtalk/token.rs,dingtalk/stream.rs}`、`src-tauri/src/{slack/client.rs,slack/ws.rs}`（base URL env 覆盖）。
- 改：`docs/specs/popup-launch-performance.md` §7（更新方法论：全 channel mock + 冷热双跑 + 固定基线）。
- 重采基线：`docs/perf/baseline.json`（变为 cold/warm 双组 + 含 IM attach）。

## 风险与取舍

- **mock 与协议耦合**：若未来某 channel 的 connect 握手要求新的服务端帧，mock 需同步更新，否则该 channel connect 失败 → 测试报错（属「需要关注」的真实信号，但可能是 mock 滞后而非产品劣化；用 daemon.log 区分）。
- **延迟定位**：建连与发送两处各加 ~150ms。建连延迟落在 `Router::connect()` 的 await（当前在 spawn 前）→ 进端到端，暴露「建连阻塞弹窗」；方案3 把 spawn 提到 attach 前后后，该延迟应从端到端消失（表现为改善）。发送延迟当前因 `ch.start` 异步不进端到端，作为「未来发送被改成阻塞」的探针。
- **env 覆盖仅测试用**：未设时常量不变，零生产影响。

## 与低风险优化（方案7/2/1）的关系

本 harness 是「防回归地基」。建议**先做本 harness + mock、采好冷热基线，再做方案7/2/1**，使后续每个优化都能在确定性基线上验证收益与防回归。方案7/2/1 计划见 `docs/plans/popup-launch-low-risk-optimization.md`。
