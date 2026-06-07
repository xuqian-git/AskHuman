# 计划：引入常驻 Daemon 架构

> 关联需求：`docs/specs/daemon-architecture.md`
> 分支：`feat/daemon-architecture`

本计划把架构改造拆成可独立验证、每步保持编译通过的阶段。核心思路：**先把最容易出错的进程间管道（CLI↔Daemon↔GUI）用「纯弹窗」打通，再把 IM 渠道迁进 Daemon，最后补实时配置与生命周期细节。**

## 0. 总览：新增 / 改动模块

```
src-tauri/src/
  ipc/                      [新] 进程间通信
    mod.rs                  协议消息类型（serde）、protocol_version
    codec.rs                NDJSON 编解码（按行读写 JSON）
    transport.rs            跨平台传输抽象（Unix socket / Windows named pipe）
  daemon/                   [新] 常驻进程
    mod.rs                  daemon 子命令分发（run/start/stop/restart/status/logs）
    lifecycle.rs            flock 单实例 / daemon.json / 二进制指纹 / 空闲退出 / drain
    spawn.rs                自启（detach：Unix double-fork+setsid / Windows DETACHED_PROCESS）
    server.rs              IPC Server：accept CLI + GUI 连接、按消息分发
    request.rs             RequestManager：每 request_id 一套 Coordinator + Channel 任务
    router.rs              长连接 Router：out_track_id / user_id → 请求
    config_watch.rs        notify 监听 config.json → 重载 + 重连 + 通知 GUI
  client/                   [新] CLI 作为 Daemon 客户端
    mod.rs                 连接/握手/换新重连、提交请求、转发 stdout/stderr、退出码
  cli/
    mod.rs                 [改] argv 分发：新增 daemon 子命令 + 隐藏 --popup；ask 走 client
  app/
    mod.rs                 [改] 拆出「GUI Helper 模式」窗口创建；emit_result 供 Daemon 复用
    coordinator.rs         [改] 与退出动作解耦，结果回传 IPC 而非直接 app.exit
  channels/
    popup.rs               [改] 改为 spawn GUI Helper 的 adapter（不再进程内开窗）
    telegram.rs / dingding.rs / feishu.rs   [改] 在 Daemon 内运行，长连接经 Router 复用
  dingtalk/stream.rs        [改] 由 Daemon 独占持有，单连接 + Router 分发
  feishu/ws.rs              [改] 同上
```

> 复用而非重写：`channels/conversation.rs`（`run_conversation` / `MessagingChannel`）、`channels/mod.rs`（`Channel` / `Preemption`）、`app/mod.rs::emit_result`、`cli/output.rs`、`cli/image_writer.rs`、`cli/file_attachment.rs`、`models.rs` 基本保留，主要改「在哪运行 + 结果怎么流出去」。

## 1. Phase 0：IPC + Daemon 骨架（无渠道）

目标：能 `askhuman daemon run` 起一个空 Daemon，CLI 能连上并完成握手；`status/stop` 可用。

- `ipc/`：定义 §6 的消息类型与 `protocol_version`；NDJSON codec；transport 抽象（先 Unix，Windows 跟上）。
- `daemon/lifecycle.rs`：`flock(daemon.lock)` 单实例；写/读 `daemon.json`；`current_exe()` 指纹（mtime+size）；空闲计时器（占位）。
- `daemon/spawn.rs`：detach 自启。
- `daemon/server.rs`：accept + 握手（`hello`/`helloAck`，校验 protocol + 指纹，必要时回 `restarting`）。
- `cli/mod.rs`：新增 `daemon` 子命令；`client/` 实现连接/握手/自启逻辑。
- `daemon status`：打印 running/pid/version/uptime/socket/requests/channels/config。

验证：起停、并发起只活一个、stale socket 清理、指纹变化触发换新。

## 2. Phase 1：纯弹窗 ask 经 Daemon + GUI Helper 跑通

目标：`AskHuman "问题?"`（仅弹窗）全程经 Daemon，与现状行为一致。这是最关键的管道验证。

- `cli/`：ask 路径改为 `client`：解析入参（`-f` 在此解析为绝对路径，缺失即退 1）→ 捕获 source/lang/is_markdown → `submit`。
- `app/mod.rs`：拆出「GUI Helper 模式」——`--popup --endpoint --token` 启动，仅创建弹窗、连 Daemon、收 `show`/发 `answer`/收 `cancel`、收 `configChanged`。窗口外观自读 config。
- `channels/popup.rs`：改为 adapter，在 Daemon 内 spawn GUI Helper 并桥接其 `answer` 进 `Channel`/`MessagingChannel`。
- `daemon/request.rs`：建 Coordinator，仅挂 popup adapter；收尾跑 `emit_result`（落盘 + 渲染）→ 回 `final{stdout, exitCode}`。
- `coordinator.rs`：结果不再 `app.exit`，改为回传 IPC；退出动作交给 CLI。
- CLI 断开 → Daemon 取消请求、关 GUI Helper。

验证：单题/多题、选项/文字/图片/文件、取消、被信号中断、退出码 0/1/3 均与现状一致。

## 3. Phase 2：IM 渠道迁入 Daemon + 长连接复用（解决 TODO#1）

目标：IM 长连接由 Daemon 独占、经 Router 分发；并发提问不串扰。

- `daemon/router.rs`：钉钉单 Stream 由 Daemon 持有；卡片回调按 `out_track_id`、聊天消息按 `user_id` 路由到对应请求；3 秒 ACK 由 Daemon 处理（空 ACK + OpenAPI updateCard 灰显）。
- `dingtalk/stream.rs`：从「每会话一条」改为「Daemon 全局一条 + 订阅 + 分发」。
- `channels/dingding.rs`：`ask_question` 的事件来源从自有 `StreamConn` 改为「向 Router 注册 out_track_id/user_id，收转发事件」。
- 钉钉跑通后，同法迁 `feishu/ws.rs` + `channels/feishu.rs`；再迁 `channels/telegram.rs`（Telegram 无单连接限制，但为统一也迁入 Daemon）。
- 「自动识别 userId」流程（钉钉/飞书）也经 Daemon 长连接。

验证：§9 验收 2/3（并行抢答 + 并发不串扰）。逐个渠道迁移、每步保持其余渠道可用。

## 4. Phase 3：配置实时生效 + 生命周期细节

- `daemon/config_watch.rs`：`notify` 监听 `config.json`（去抖、处理原子写/rename）→ 重载 `AppConfig` → 比对差异重连/增删 IM 连接 → 向活动 GUI Helper 发 `configChanged`。
- 空闲退出（5 分钟）落地：无活动请求且无客户端才计时；退出前发 WS Close、清理锁/socket。
- 版本/指纹自动换新打磨：`ASKHUMAN_DAEMON_AUTORESTART` 开关；drain 语义；多 CLI 同时触发不重启风暴。
- 临时目录清理：Daemon 启动时 + 定期清过期 `temp/askhuman/<request_id>/`。
- `daemon stop/restart` 优雅 drain。

验证：§9 验收 4/5/6/7/8。

## 5. Phase 4：文档与收尾

- 更新 `docs/overview.md`（已随本计划一并更新「运行模型」「目录结构」「运行流程」）。
- 用户向文档 `docs/wiki/`：如需新增「daemon 管理命令」说明则补；`AskHuman --agent-help` 文案核对。
- 旧「同 client-id 多开长连接互抢」问题：由 daemon 架构解决。
- 全量 `cargo test`、`pnpm build`、`./scripts/install.sh` 走查；并发提问手测。

## 6. 测试策略

- 单元：`ipc/codec`（NDJSON 往返）、`lifecycle`（指纹比对、daemon.json 读写）、`router`（out_track_id/user_id 路由 + 幂等丢弃）。
- 集成：起一个临时 socket 的 Daemon，模拟 CLI 提交 + GUI answer，断言 `final` 输出与退出码；模拟 CLI 断开断言取消。
- 跨平台：Unix socket 与 Windows named pipe 各跑传输用例。
- 回归：现有 `cargo test` 全过（config/cursor_hook/help/image_writer/file_attachment/card 等不回归）。

## 7. 风险与回退

- 这是大改；按 Phase 切小步、每步保持编译与现有测试通过，便于二分定位。
- 若 Phase 2 某渠道迁移受阻，可暂保留该渠道「进程内旧路径」与 Daemon 路径并存，最后统一切换（避免一次性大爆炸）。
- 钉钉 3 秒 ACK / 卡片灰显改走 OpenAPI 需尽早实测（Phase 2 最先验证）。

## 8. 提交粒度（建议）

- 每个 Phase 一个或多个小 commit，message 用英文。
- Phase 0/1 是地基，建议合入前重点 review IPC 协议与退出码/ stdout 洁净契约。
