# 需求：发布/被依赖 + Channel 粒度降级

> 状态：已确认（待按计划实现）
> 关联计划：`docs/plans/release-and-channel-degradation.md`

## 1. 背景

`HumanInLoop` 是 Tauri 2（Rust + Vue3）应用，产物为单一可执行文件 `AskHuman`：既是 CLI，又能按需弹出 GUI 窗口收集人类回应；同时支持多「通信 Channel」（本地弹窗 popup + Telegram）并行「抢答」。

当前发布状况的不足：

- CI（`.github/workflows/build.yml`）用 `cargo build` 产出**裸二进制 artifact**（90 天过期），无 tag、无正式 Release、无包管理分发。
- 版本号分散在 `Cargo.toml` / `tauri.conf.json` / `package.json` 三处，手动维护易漂移。
- 没有「被其他项目依赖」的能力。

## 2. 目标

本需求要解决两件事：

### 需求 A：可发布、可被依赖

`AskHuman` 需要同时满足两种消费方式：

1. **单独使用**：用户 `npm i -g @humaninloop/cli` 后在终端直接用 `AskHuman ...`；或从 GitHub Release 下载。
2. **被其他库依赖**：典型下游为 `WeiboLongRunningAgent`(WBLRA) 这类 Node/TS 项目，把 `humaninloop` 写进 `dependencies`，用户 `npm install` 时**自动装上对应平台二进制**，运行时在代码里解析路径并 `spawn` 调用。

### 需求 B：Channel 粒度的优雅降级

「可用性」必须是 **Channel 粒度**，而非整体开关：

- 本地弹窗（GUI）打不开时（典型：Linux 缺 WebKitGTK、headless 无显示环境），**给出明确报错信息**；
- 但只要配置了 Telegram，**仍能通过 Telegram channel 正常完成提问**；
- 只有当**所有 channel 都不可用**时，才算真正不可用，并以专门退出码告知下游，便于下游降级（如跳过人工确认而不阻塞流程）。

> 现状问题：当前 Telegram channel 寄生于 Tauri 事件循环，且 `app::launch` 在 `tauri::Builder::build()` 处 `.expect("启动 Tauri 失败")`。一旦 GUI 子系统初始化失败会直接崩溃，**即使配了 Telegram 也一起挂掉**——与本需求相反。

## 3. 已确认决策（汇总）

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 主分发渠道 | npm「平台子包」（esbuild/biome 同款） |
| D2 | 补充分发渠道 | 保留 GitHub Release 的 tar.gz / zip |
| D3 | 不采用的方案 | Homebrew（只解决「人来装」，不解决「被库依赖」）；axe 式 bundle 捆绑（本项目单文件，无需下游捆绑） |
| D4 | 主包名 | `@humaninloop/cli`（scoped；非 scoped 的 `humaninloop` 被 npm 判定与 `human-in-loop` 太像而拒绝） |
| D5 | 平台子包名 | scoped（esbuild 风格，规避 npm 反垃圾）：`@humaninloop/darwin-arm64` / `@humaninloop/darwin-x64` / `@humaninloop/win32-x64` / `@humaninloop/linux-x64` |
| D6 | registry | 公共 npmjs |
| D7 | 运行时 API | 主包导出 `getBinaryPath()` / `isAvailable()`；解析顺序：环境变量 `HUMANINLOOP_BINARY` → 平台子包 → 系统 `PATH` |
| D8 | 退出码契约 | `0`=成功或用户取消；`3`=无任何可用 channel（需降级）；`1`=其他异常 |
| D9 | 降级语义 | Channel 粒度（见需求 B） |
| D10 | Linux 支持 | 发布 linux 子包；文档注明需系统 WebKitGTK；`isAvailable()` / 运行时探测在缺失时降级 |
| D11 | 覆盖平台 | macOS arm64、macOS x64、Windows x64、Linux x64 |
| D12 | 首发版本号 | `0.1.0`（保持现状，pre-1.0） |
| D13 | CI 处置 | 保留 `build.yml` 作 CI 校验（PR/push 编译验证，不发布）；新增 `release.yml` 由 tag 触发发布 |
| D14 | GUI 可用性判定 | 轻量预探测（Linux 检测 `DISPLAY`/`WAYLAND_DISPLAY` 与 WebKitGTK 可加载性）+ `tauri::Builder::build()` 返回 `Err` 双重判定；不可用且 Telegram 已配置则转 headless 走 Telegram |
| D15 | 文档落位 | spec → `docs/specs/`；plan → `docs/plans/` |

## 4. 约束与既有规则（不可破坏）

- **stdout 洁净**：stdout 只输出结果区块（`[选择的选项]`/`[用户输入]`/`[图片]`/`[状态]`），所有日志/报错走 stderr。被依赖时下游解析 stdout，契约不能变。
- **release 为 `panic = "abort"`**（见 `src-tauri/Cargo.toml`）：因此**不能依赖 `std::panic::catch_unwind`** 捕获 GUI 初始化崩溃，降级判定必须走「可返回的错误路径」（预探测 + `Result`）。
- **前端资源在 `cargo build` 时由 `generate_context!` 嵌入二进制**：产物自包含、单文件，无外部 framework（区别于 axe）。
- **npm 不安装系统库**：Linux 的 WebKitGTK 属系统依赖，由用户用 apt/dnf 安装，npm 包只负责二进制本身。
- 现有功能契约保持：多 channel「抢答」（首个终态结果生效、其余收尾）、配置文件 `~/.humaninloop/config.json`、`--settings`/`--help`/`--version` 行为不变。

## 5. 验收标准

需求 A：

1. 打 tag `vX.Y.Z` 后，CI 自动：编译 4 平台二进制 → 发布 4 个平台子包与主包到 npmjs → 上传 tar.gz/zip 到 GitHub Release。
2. 在 macOS/Windows/Linux 任一机器 `npm i -g @humaninloop/cli` 后，`AskHuman` 命令可用，仅安装当前平台对应的一个子包。
3. 下游项目把 `humaninloop` 加入 `dependencies` 并 `npm i` 后，可通过 `getBinaryPath()` 拿到二进制路径并成功 `spawn`；`isAvailable()` 正确反映是否就位。
4. 三处 + 子包版本号在一次发版内保持一致。

需求 B：

5. Linux 无 WebKitGTK / headless 环境下，**配置了 Telegram 时**：不崩溃，stderr 打印本地弹窗不可用的原因，并通过 Telegram 完成提问、stdout 正常输出结果区块、退出码 0。
6. 同样环境下**未配置 Telegram 时**：不崩溃，stderr 打印明确原因，退出码 `3`。
7. 正常桌面环境下行为与现状一致（弹窗 + 可选 Telegram 抢答）。

## 6. 反馈意见

（后续 review 中产生的调整意见追加于此，标注日期。）
