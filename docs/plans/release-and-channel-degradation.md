# 开发计划：发布/被依赖 + Channel 粒度降级

> 关联需求：`docs/specs/release-and-channel-degradation.md`
> 分两批：批次 A（发布与被依赖，npm 打包 + CI）/ 批次 B（Channel 降级，Rust 解耦）。
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
git tag vX.Y.Z
  └─► CI release.yml（tag 触发）
        ├─ 4 平台编译 AskHuman 二进制
        ├─ 填充各平台子包 → npm publish（@humaninloop/darwin-arm64 等 4 个）
        ├─ npm publish 主包 @humaninloop/cli（optionalDependencies 锁定子包版本）
        └─ 打 tar.gz/zip → 创建 GitHub Release 并上传

消费①（单独）：npm i -g @humaninloop/cli → AskHuman ...     或 GitHub Release 下载
消费②（被依赖）：下游 dependencies 加 humaninloop → npm i 自动装当前平台子包
                运行时：getBinaryPath() 解析路径 → spawn → 读 stdout / 退出码
```

二进制运行期的 Channel 决策（批次 B）：

```
是否需要 popup？(config.channels.popup.enabled)
  ├─ 否 + Telegram 可用 ───────────► headless：仅 tokio 跑 Telegram（不进 Tauri）
  ├─ 是 + GUI 预探测/build 成功 ──► 现行为：弹窗(+可选 Telegram) 抢答
  ├─ 是 + GUI 不可用 + Telegram 可用 ► stderr 报原因 + 转 headless 走 Telegram
  └─ 任一路径下「无任何可用 channel」 ► stderr 报原因 + 退出码 3
```

---

## 批次 A：发布与被依赖（npm 打包 + CI）

### A1. 仓库内 npm 包目录结构

新增 `packaging/npm/`（编译产物由 CI 注入，不提交二进制）：

```
packaging/npm/
  humaninloop/                      主包（薄、跨平台）
    package.json
    index.js                        导出 getBinaryPath() / isAvailable()
    bin/cli.js                      JS shim：解析平台二进制并 spawn 透传
    README.md  LICENSE
  platforms/
    darwin-arm64/package.json       子包模板（bin/AskHuman 由 CI 放入）
    darwin-x64/package.json
    win32-x64/package.json
    linux-x64/package.json
```

`.gitignore` 增加 `packaging/npm/**/bin/AskHuman` 与 `*.exe`，子包二进制只在 CI 内填充。

### A2. 主包 `humaninloop/package.json` 关键字段

- `"name": "humaninloop"`，`"version": "0.1.0"`，`"license": "MIT"`。
- `"bin": { "AskHuman": "bin/cli.js" }`：满足全局命令（Windows 由 npm 自动生成 `.cmd`）。
- `"main": "index.js"`，`"type": "commonjs"`。
- `"optionalDependencies"`：四个子包，版本与主包**完全一致**（CI 锁定）。
- `"files": ["index.js", "bin", "README.md", "LICENSE"]`。
- 不放二进制（二进制全在子包）。

### A3. 平台子包 `humaninloop-<os>-<cpu>/package.json` 关键字段

- `"name"`：如 `@humaninloop/darwin-arm64`（scoped）。
- `"os": ["darwin"]`、`"cpu": ["arm64"]`：npm 据此**只装匹配当前平台的一个**。
- `"files": ["bin"]`，二进制位于 `bin/AskHuman`（Windows 为 `AskHuman.exe`）。
- npm `os`/`cpu` 取值映射：darwin/win32/linux × arm64/x64。

### A4. 运行时 API（主包 `index.js`）

导出两个函数（对齐 WBLRA 的 `resolveAxeBinary` 风格）：

- `getBinaryPath()`：按顺序解析并返回二进制绝对路径——
  1. 环境变量 `HUMANINLOOP_BINARY`（存在且可执行则用之，便于测试/自定义）；
  2. 当前平台子包：用 `require.resolve("humaninloop-<os>-<cpu>/bin/AskHuman")` 定位；
  3. 系统 `PATH` 中的 `AskHuman`。
- `isAvailable()`：上述解析能得到一个**存在且可执行**的路径则 `true`，否则 `false`。
  - 说明：这是「二进制是否就位」的粗粒度判断；具体哪个 channel 能用由二进制运行期决定（见批次 B 退出码契约）。

平台键由 `process.platform` + `process.arch` 推导（`win32`/`darwin`/`linux` × `arm64`/`x64`）。

### A5. 主包 bin shim（`bin/cli.js`）

- 调 `getBinaryPath()` 取二进制；找不到则 stderr 提示「未安装当前平台二进制」并以非 0 退出。
- `spawn(bin, process.argv.slice(2), { stdio: "inherit" })`，**透传退出码**给调用方。
- 仅服务「全局命令」场景；下游程序集成应直接用 `getBinaryPath()`，不经此 shim。

### A6. 下游接入范式（写入主包 README，供 WBLRA 参考）

```js
import { getBinaryPath, isAvailable } from "humaninloop";
import { spawnSync } from "node:child_process";

if (!isAvailable()) { /* 未安装：跳过人工确认环节 */ }
else {
  const r = spawnSync(getBinaryPath(), ["要继续吗？", "-o", "继续", "-o", "停止"], { encoding: "utf8" });
  if (r.status === 3) { /* 环境无可用 channel：降级，不阻塞流程 */ }
  else if (r.status === 0) { /* 解析 r.stdout 的结果区块 */ }
}
```

### A7. `release.yml`（tag 触发的发布流水线）

- 触发：`on.push.tags: ['v*']`。
- 复用现有 `build.yml` 的 4 平台矩阵与 Linux WebKitGTK apt 依赖、pnpm/node/rust 步骤。
- 各 job 步骤：`pnpm install` → `pnpm build`（前端）→ `cargo build --release --target <triple>` → 产出 `AskHuman[.exe]`。
- 汇总 job：
  1. 下载各平台产物，按 `os-cpu` 放入对应子包 `bin/`；
  2. 写入版本号（见 A9）；
  3. `npm publish` 4 个子包，再 `npm publish` 主包（顺序：先子包后主包）；
  4. 打包 `AskHuman-<triple>-vX.Y.Z.tar.gz`（Windows 用 zip），创建 GitHub Release（用内置 `GITHUB_TOKEN`）并上传。
- 发布鉴权：需在仓库 Secrets 配置 `NPM_TOKEN`（前置条件，见 A10）。
- 产物命名规范：
  - GitHub Release：`AskHuman-<rust-triple>-vX.Y.Z.{tar.gz|zip}`（triple 与 build.yml 现用一致）。
  - npm 子包内：统一 `bin/AskHuman`（win 为 `AskHuman.exe`）。

### A8. CI 校验（保留 `build.yml`）

- 维持 PR / push / 手动触发；职责改为「编译验证」（不再上传可发布 artifact，不创建 Release）。
- 可保留裸二进制 artifact 供调试，但不作为正式分发入口。

### A9. 版本号同步（`scripts/bump-version`）

- 以传入的版本号为单一事实来源，一次写入：`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、根 `package.json`、主包 `packaging/npm/humaninloop/package.json`、4 个子包 `package.json`，以及主包 `optionalDependencies` 中对子包的版本锁定。
- 发版流程：运行脚本 → 提交 → 打 tag `vX.Y.Z` → push tag 触发 `release.yml`。
- `AskHuman --version` 文案来源与 `Cargo.toml` 版本对齐（核对 `cli/help.rs`）。

### A10. 前置条件与文档

- 前置：在 GitHub 仓库 Secrets 配置 `NPM_TOKEN`（npmjs automation token）。
- README 更新：新增「npm 安装（单独使用）」「被依赖（下游集成）」两节；把「从 Actions artifact 下载」改为指向 GitHub Release；Linux 注明需 WebKitGTK。

---

## 批次 B：Channel 粒度降级（Rust 解耦）

### B1. 现状与问题（定位）

- `app/mod.rs::launch()` 是统一启动入口（`-> !`）：无论是否需要弹窗，都 `tauri::Builder::default()...build().expect("启动 Tauri 失败")` 进入 Tauri 事件循环。
- `channels/telegram.rs` 用 `tauri::async_runtime::spawn` 跑长轮询，结果投递给 `Coordinator`。
- `app/coordinator.rs::Coordinator` 持有 `AppHandle`，`submit()` 末尾 `app.exit(code)` 退出。
- 因此：GUI 初始化失败 → `.expect` panic（release 下 `panic=abort` 直接终止）→ Telegram 一并失效。

### B2. 解耦目标

让「结果协调 + 输出 + 退出」与「Tauri/GUI」解耦，使 Telegram 能在**不进 Tauri 事件循环**的 headless 路径下独立工作。

### B3. Coordinator 去 GUI 化

- 把 `Coordinator` 对 `AppHandle` 的硬依赖改为「退出策略」抽象：
  - GUI 模式：拿到结果 → `emit_result` → `app.exit(code)`；
  - headless 模式：拿到结果 → `emit_result` → `std::process::exit(code)`。
- `emit_result`（输出区块 + 计算退出码）已与退出动作可分离，保持复用；`PopupChannel`/`TelegramChannel` 的 `Channel` trait 接口不变。
- `Coordinator::register/submit/抢答` 语义不变（首个终态生效、其余 `cancel_by_other`）。

### B4. headless 运行路径（新增）

- 当判定走 headless（见 B5 决策树）时：
  - 自建一个 tokio 多线程 runtime（`Cargo.toml` 已有 `tokio` 多线程 feature），在其上跑 `TelegramChannel` 的会话逻辑与协调器；
  - 不调用 `app::launch`、不创建任何窗口、不进入 Tauri 事件循环；
  - **不调用 `stderr_redirect::silence()`**（该静默仅为 GUI 噪音设计）；headless 下 stderr 正常输出报错与降级原因。
- Telegram 会话逻辑（`run_session`）应可在「普通 tokio runtime」上运行（当前用 `tauri::async_runtime::spawn`，需改为 runtime 无关的 spawn / 直接 await），以便 GUI 与 headless 两条路径共享同一会话实现。

### B5. Channel 决策树（在创建窗口前判定）

入口在 `cli::dispatch` 的提问分支 / `app::run_ask` 之前，按下列顺序：

1. 计算 `telegram_active`（沿用现逻辑：`enabled && TelegramClient::new(...).is_ok()`）。
2. 计算 `popup_wanted = config.channels.popup.enabled`。
3. 判定 `gui_available`（见 B6）。
4. 分支：
   - `popup_wanted == false && telegram_active` → headless（仅 Telegram）。
   - `popup_wanted == true && gui_available` → GUI 路径（弹窗 + 若 telegram_active 则并行）。
   - `popup_wanted == true && !gui_available && telegram_active` → stderr 输出「本地弹窗不可用：<原因>」→ headless（仅 Telegram）。
   - 其余（无任何可用 channel：GUI 不可用且 Telegram 未配/不可用） → stderr 输出明确原因 → 退出码 `3`。
   - 兜底：`popup_wanted == false && !telegram_active`（理论不应到此）→ 视为无可用 channel → 退出码 `3`（替代当前「兜底强开弹窗」，因为该兜底在无 GUI 环境会崩）。

### B6. GUI 可用性判定（`gui_available`）

因 `panic=abort` 不能 `catch_unwind`，采用「预探测 + Err 双重判定」：

- 预探测（进入 Tauri 前）：
  - macOS / Windows：默认 `true`（系统 WebView 常驻）。
  - Linux：要求存在 `DISPLAY` 或 `WAYLAND_DISPLAY`，且 WebKitGTK 可加载（运行期 `dlopen` 探测 `libwebkit2gtk-4.1.so`，失败视为不可用）。
- Err 判定：实际进入 GUI 路径时，将 `tauri::Builder::build()` 的 `.expect(...)` 改为 `match`，返回 `Err` 即按「GUI 不可用」处理，回退至 B5 的对应分支（有 Telegram 转 headless，否则退出码 `3`）。
- 实现首个任务（验证项）：实测 Linux 无 WebKitGTK 时 `build()` 的失败形态（返回 `Err` 还是进程 abort）。若为 abort，则**必须**依赖上面的 Linux 预探测拦截，避免进入 `build()`；预探测即为此而设。

### B7. 退出码契约（落地到代码）

- `0`：成功（任一 channel 拿到 Send 结果）或用户取消（输出 `[状态]`）——与现状一致。
- `3`：无任何可用 channel（GUI 不可用且 Telegram 未配/不可用）。**新增常量**。
- `1`：其他异常（如图片落盘失败等，沿用现状）。
- stderr：在退出码 3 与「弹窗不可用但回退 Telegram」时，输出明确、可读的中文原因。

### B8. 涉及文件（预估）

- `src-tauri/src/app/mod.rs`：`launch` 拆分；`build()` 不再 `expect`；GUI 路径与 headless 路径分流；`emit_result` 复用。
- `src-tauri/src/app/coordinator.rs`：退出策略抽象（GUI/headless）。
- `src-tauri/src/channels/telegram.rs`：会话 spawn 改为 runtime 无关，支持 headless 复用。
- `src-tauri/src/cli/mod.rs` 或新增 `app` 子模块：Channel 决策树入口、GUI 预探测。
- 新增退出码常量与 GUI 探测的小模块（位置实现时定）。

---

## 任务清单与顺序

批次 A（可独立先行）：

1. A1 目录骨架 + `.gitignore` 调整。
2. A2/A3 主包与 4 子包 `package.json`。
3. A4 `index.js`（`getBinaryPath`/`isAvailable`）+ A5 `bin/cli.js`。
4. A9 `scripts/bump-version` + 版本对齐核对（含 `help.rs`）。
5. A7 `release.yml`；A8 调整 `build.yml` 职责。
6. A10 README/文档更新；记录 `NPM_TOKEN` 前置条件。
7. 演练：本地用 `npm pack` + 模拟平台子包验证解析与 spawn。

批次 B（依赖对 Channel 架构的改动，建议 A 之后或并行）：

8. B6 验证 Linux GUI 失败形态（首个验证任务）。
9. B3 Coordinator 退出策略抽象。
10. B4 headless 运行路径 + B5 决策树 + B6 预探测。
11. B7 退出码常量与 stderr 原因输出。
12. 回归：桌面环境行为不变；构造「无 GUI + 有/无 Telegram」两种环境验证降级与退出码。

## 风险与验证

- **GTK 失败形态不确定**（B6 首个任务）：若为不可捕获的 abort，则强依赖 Linux 预探测；预探测需覆盖「无 DISPLAY」「有 DISPLAY 但无 WebKitGTK」两种。
- **npm 平台子包解析**：`require.resolve` 在下游 `node_modules` 提升/嵌套布局下需稳定；以 `npm pack` 本地演练验证。
- **退出码语义变更**：新增 `3` 不得与现有 `0/1` 冲突；需同步更新 README 输出契约说明。
- **版本漂移**：所有发版必须经 `scripts/bump-version`，禁止手改单处版本。

## 测试策略

- Rust：为 Channel 决策树与退出码映射加单元测试（纯函数化判定，便于覆盖四个分支）；保留现有 `config`/`output` 等单测。
- npm：`index.js` 平台键推导与解析顺序的最小单测 + `npm pack` 安装演练。
- 端到端：tag 预发布（如 `v0.1.0-rc.1`）跑通 `release.yml`，在三平台实测安装与被依赖。
