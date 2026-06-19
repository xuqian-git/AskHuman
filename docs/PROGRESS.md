# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 弹窗启动延迟性能优化（埋点 + harness + 基线 + 首轮 + 次轮已落地 → 余方案6/8/markdown-it，远期）

文档：`docs/specs/popup-launch-performance.md`（调用链、等待点、优化方案、度量方法论 §7）。
harness 计划：`docs/plans/perf-harness-deterministic-mock-im.md`。
优化计划：`docs/plans/popup-launch-low-risk-optimization.md`（首轮 1/2/7）、`docs/plans/popup-launch-daemon-optimization.md`（次轮 3/4/5）。

**已完成：埋点 + 确定性 harness**（`ASKHUMAN_PERF` 门控默认关；`scripts/perf-popup.mjs` 无脑单命令：隔离 daemon
+ `ASKHUMAN_NO_KEYCHAIN=1` + 全 4 渠道 mock IM（`perf-mock-im.mjs`，建连/发送各注入 ~150ms 探针）+ 冷热双跑
+ 端到端 p90 ±20% 回归闸 + 锁屏/息屏守卫；基线 `docs/perf/baseline.json` 含 cold/warm）。

**已完成：首轮（方案1/2/7 + 支撑 S）** —— 前端侧：main.ts 不阻塞挂载、PopupView.onMounted 先取内容渲染、
Settings/History/Agents 异步组件、popup_init 作弹窗唯一非钥匙串配置源（弹窗路径零 `get_settings()`）；
附带 HistoryView 改用 `history_init.lang`，main.ts 自此零 IPC。

**已完成：次轮（方案3/4/5）** —— daemon/CLI 侧：
- 方案3 daemon 提前 spawn 弹窗（移到 Accepted 后、attach/inbound 前）→ WebView 初始化与 IM 建连并行。
- 方案4 attach/inbound 用 `any_im_enabled`(`load_without_secrets`) 门控，无启用 IM 时跳过 `AppConfig::load()`（零钥匙串）。
- 方案5(b) detect 移 daemon 异步：CLI 只读 env 家族/会话 + 上送 `caller_pid`；daemon spawn 弹窗后独立 task 从
  caller_pid walk 出家族/pid（MCP `walk_any` 兜底），经新 `ServerMsg::AgentResolved` 后推弹窗 badge（缓存 + 事件
  + 握手补发覆盖竞态）。badge 端到端验证通过（本仓 AskHuman 弹窗显 cursor 且可点 ↗）。

**当前基线**（`docs/perf/baseline.json`，次轮后 `--update-baseline` 刷新，屏幕解锁+唤醒+勿遮挡下采）：
- COLD 端到端 p90 ≈ **578ms**（首轮后为 ~1188）：方案3 让 `daemon recv→spawned` 466→1ms，~467ms IM 建连现与弹窗并行、不再进端到端。
- WARM 端到端 p90 ≈ **520ms**（首轮后 ~583）：大头仍是 `GUI total show→painted` ≈496（window visible ~250 + page boot ~435），即 WebView/页面加载固有冷成本。
- CLI `detect` 两路均 ~1ms（方案5：原 COLD ~39 / WARM ~27ms 的 ps 游走已离开 CLI）。

**余下**：方案8 延后 show/骨架屏（改观感不减时长，热路径已并入方案6）、markdown-it 仅 `isMarkdown` 时按需懒加载（见 spec §4/§6）。

### 进行中：方案6 弹窗预热（进程池）—— 代码已落地+编译/装机/机制验证通过；待解锁采基线 + 真机视觉 sanity + 提交

需求 `docs/specs/popup-prewarm.md` + 计划 `docs/plans/popup-prewarm.md`。
形态：daemon 预热 1 个 `--popup --warm` helper（已挂载、无请求隐藏待命），来请求由 `dispatch_popup` 领用喂
`Show` 直接上屏、用后后台重建；**默认开**可关、非实验；并发第 2+/无显示/未就绪/drain 透明回退现有冷 spawn；
热连接非保活、idle/换新回收重补。

**已落地（编译 + install 通过）**
- 阶段1 config `general.popupPrewarm`（默认开）+ 设置页开关 + i18n。
- 阶段2 helper `--warm` 角色；前端待命/`popup-show` 领用注入/渲染；窗口 `background_throttling(Disabled)`。
- 阶段3 daemon 热池（`warm_pool`/`WarmSlot`）+ `handle_gui_warm` + `serve_gui`（冷热共用尾段）+ `dispatch_popup` 领用 + 冷 spawn 完整回退。
- 阶段4 补热 `maybe_topup_warm`（启动/领用后/死亡后/开关开）+ `has_display`（headless 不预热）+ 生命周期
  （热连接非保活、idle/排空/换新 `recycle_warm` 重补）。
- 阶段5 perf 透传：`ShowPayload` 带 `perf_id`/`perf_autodismiss`，热 helper 领用时 `perf::set_runtime` 开埋点
  （无 env 也能量化）；harness 增 **`hot`** 档（prewarm 开、热路径 only 聚合、`daemon recv→assigned` 行）；
  cold/warm 档显式 `popupPrewarm=false` 保持与旧基线可比。

**关键修正（验证中发现）**：原设计「隐藏窗双 rAF 绘制完成才 show」**不可行**——窗口 ordered-out 无 display link，
rAF 根本不回调（`background_throttling(Disabled)` 只防节流、不为隐藏窗驱动帧）。改为**领用时 `nextTick` 等 DOM
更新好正文后直接后端 `popup_show_window` 上屏**（show 不再依赖 rAF），rAF/打点放到 show 之后。实测：**息屏/锁屏下
弹窗照常上屏**（`gui.win_show` 触发），仅 show 后的 `fe.painted`/harness autodismiss 因无刷新而暂停（只影响 harness，
其本就拒绝锁屏运行）。无「加载中→正文」闪现（show 时 DOM 已是正文）。

**已验证**：daemon 重启日志 `preheating popup helper`/`warm popup ready`；隔离机热路径端到端 `dmn.assigned`→
`gui.show_recv`→`gui.win_show`→`fe.painted`（解锁 ~212ms exit；息屏 show 仍触发）；GUI-free 生命周期（开→1 热进程、
关→回收 0、再开→重补 1、停 daemon→自杀 0）。

**待办**：① 解锁+唤醒+勿遮挡下跑 `node scripts/perf-popup.mjs --update-baseline` 采三档基线（含 hot）；
② 真机视觉 sanity（连弹/并发第2 冷回退/改主题/drain 换新；headless 仅 Linux）；③ 文档收尾 + 提交。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
