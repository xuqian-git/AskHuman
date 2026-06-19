# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 进行中：弹窗启动延迟性能优化（埋点 + harness 已落地，优化方案待做）

文档：`docs/specs/popup-launch-performance.md`（完整调用链、等待点清单、优化方案、度量方法论 §7）。

已完成：
- **埋点**（`ASKHUMAN_PERF` 门控，默认关、零开销）：`src-tauri/src/perf.rs` + CLI/daemon/helper/前端 16 个里程碑，
  统一写 `~/.askhuman/perf.log`（`<epoch_ms>\t<perf_id>\t<stage>\t<pid>`），按 `perf_id` 串联整条时间线。
- **harness**：`scripts/perf-popup.mjs`——零交互（弹窗画完首帧自动取消）跑 N 次、聚合中位/p90、
  存/比基线、端到端 p90 超阈（默认 20%）退出码 1。
- 已 `install.sh` 装好并实测：端到端热路径 ~0.55s，GUI/页面加载占 ~90%（基线样例见文档「附」）。

下一步（优化方案，尚未动手；需先写 plan / 确认再改）：方案1（`main.ts` 取消挂载前 `await getSettings`）、
方案2（`onMounted` 先 `popupInit`）、方案3（daemon 提前 spawn helper）、§5（detect 移到 daemon 侧）。
用 `node scripts/perf-popup.mjs --save-baseline docs/perf/baseline.json` 先固化正式基线再逐方案对比防回归。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
