# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 进行中：弹窗启动延迟性能优化（埋点 + 确定性 harness 已落地，待采基线 → 再做优化方案）

文档：`docs/specs/popup-launch-performance.md`（调用链、等待点、优化方案、度量方法论 §7）。
harness 计划：`docs/plans/perf-harness-deterministic-mock-im.md`。优化计划：`docs/plans/popup-launch-low-risk-optimization.md`。

已完成：
- **埋点**（`ASKHUMAN_PERF` 门控，默认关、零开销）：`src-tauri/src/perf.rs` + CLI/daemon/helper/前端 ~18 里程碑，
  统一写 `~/.askhuman/perf.log`（`<epoch_ms>\t<perf_id>\t<stage>\t<pid>`），按 `perf_id` 串联整条时间线。
- **确定性 harness**（无脑单命令 `node scripts/perf-popup.mjs`，固定 canonical 场景 + 固定基线 `docs/perf/baseline.json`，
  有则比/无则建/劣化退非零，仅留 `--update-baseline`）：
  - 隔离 daemon（临时 HOME，绝不碰真实 daemon / 在途）+ `ASKHUMAN_NO_KEYCHAIN=1`（零钥匙串副作用）。
  - **本地 mock IM 全 4 渠道**（`scripts/perf-mock-im.mjs`）：建连+发送各注入 ~150ms 当「IM 阻塞弹窗」探针；
    钉钉/Slack 硬编码端点经新 env `ASKHUMAN_{DINGTALK,SLACK}_API_BASE` 指向 mock（仅测试，未设不变）。
  - **冷+热同跑**两组、各出表，基线含 `cold`/`warm`。
  - **屏幕守卫**：锁屏（`ioreg` 读 `CGSSessionScreenIsLocked`）报错不跑、`caffeinate -d` 防息屏、弹窗未上屏即中止。

**当前卡点 / 下一步**：本机此刻锁屏，无法采集真实基线（弹窗 rAF 被暂停）。需在**屏幕解锁+唤醒+勿遮挡弹窗**下跑
`node scripts/perf-popup.mjs --update-baseline` 采 cold/warm 基线写入 `docs/perf/baseline.json`，随后再做优化方案。

后续优化：首轮低风险组合 = 方案7 代码分割 + 方案2 popupInit 提前 + 方案1 main.ts 不阻塞
（见 `docs/plans/popup-launch-low-risk-optimization.md`，**待基线就绪后再改优化代码**）；
再后：方案6 预热（大头）、方案5 detect 移 daemon 等见 spec §4-6。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
