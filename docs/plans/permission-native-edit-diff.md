# 实施计划：权限弹窗展示 Agent 原生编辑 Diff

> 需求规格：`docs/specs/permission-native-edit-diff.md`（D1–D13）。
>
> 状态：已实现并于 2026-07-14 完成自动验证及 Claude Code / Codex 真实权限弹窗验收。
> 本计划保留为实现边界与验证记录，不含未决分支。

## 1. 实现原则

1. **审批关键路径零文件 I/O**：Hook adapter 和 daemon 只解析/转发 Agent 载荷；Popup 首帧后才启动读取。
2. **Popup-only**：新增 edit intent 只走 `ConfirmTask → ShowPayload → PopupInit`，不进入 `ConfirmRequest`。
3. **结构化渲染**：Rust 产出 Diff model，Vue 以文本节点渲染；不把 Diff 当 Markdown/HTML。
4. **载荷优先、快照增强**：任何 Worker 故障都保留载荷预览和现有审批行为。
5. **adapter 扩展**：UI 不判断 Agent；每家原生 schema 在 Rust adapter 内结束。
6. **有界且可终止**：所有输入、读取、算法、输出和子进程都有硬限制。

## 2. 当前实现差距

| 当前模块 | 现状 | 本功能需要 |
|---|---|---|
| `permissions.rs` | `Edit` / `Write` / `apply_patch` 只做 summary + 12k raw body；未识别 `NotebookEdit` | 严格 adapter + popup edit intent + 载荷 Diff |
| `ConfirmTask` | 只有通用 `ConfirmSpec` 与调用方上下文 | 增向后兼容 `popup_edit` |
| `ConfirmRequest` | daemon-owned、被 Popup/IM 共用 | **保持不变** |
| `ShowPayload` / `PopupInit` | 没有 Popup 专属工具数据 | 透传当前请求 edit intent |
| Popup Helper | 只接收/显示请求，不读目标文件 | 首帧后短命 Worker 编排 |
| `ConfirmPane.vue` | body 走普通 Markdown | 专用 Diff 组件 + 折叠 raw body |
| `gitutil.rs` | 生成 working tree vs index 的 `/diff` | 不可复用其 Git 读取逻辑；只复用行种类/预算思想 |

## 3. 目标模块与数据模型

新增 Rust 域目录：

```text
src-tauri/src/permission_diff/
  mod.rs        公共入口、限制常量与 model re-export
  model.rs      serde wire/domain 类型与校验
  adapters.rs   Claude/Codex 原生载荷 → PermissionEditIntent
  patch.rs      Codex apply_patch 严格 parser
  build.rs      载荷 Diff、快照增强、hunk 分组与截断
  safety.rs     路径归一化、macOS 预跳过、读取预算
  worker.rs     隐藏 Worker stdin/stdout 及父进程编排
```

新增前端文件：

```text
src/views/popup/PermissionDiffPane.vue
src/views/popup/permissionDiff.ts
```

核心 Rust 类型固定为：

```text
PermissionEditIntent
  agent_kind
  native_tool
  workspace
  operation: TextReplace | WholeFileWrite | PatchSet | Unsupported
  initial: PermissionDiffModel?
  read_targets[]

PermissionDiffModel
  request_id
  snapshot_status
  snapshot_at_ms?
  files[] / hunks[] / lines[]
  total and omitted counters
```

所有 wire 类型使用 `#[serde(rename_all = "camelCase")]`；新增字段带 `default` + `skip_serializing_if`，
保持新旧 CLI/daemon/helper 滚动切换兼容。

## 4. P1：领域模型、adapter 与载荷预览

**触点**：`src-tauri/src/main.rs`、新 `permission_diff/*`、`permissions.rs`、测试 fixtures。

### 4.1 依赖与限制

- 在 `src-tauri/Cargo.toml` 增 `similar = "2.7"`，只用默认 text 功能；
- 不使用当前 `similar 3.x`：其 MSRV 为 Rust 1.85，而项目声明 Rust 1.82；
- 行 Diff 使用 `similar::TextDiff` / grouped ops，并设置内部 deadline；
- 不启 `inline`、`unicode`、`bytes`，避免无关依赖和 word-level scope。

固定常量：

```text
MAX_WORKER_MS               = 300
MAX_FILE_BYTES              = 1 MiB
MAX_FILES                   = 64
MAX_TOTAL_READ_BYTES        = 4 MiB
MAX_DIFF_LINES_PER_FILE     = 400
MAX_DIFF_LINES_TOTAL        = 3000
MAX_TOOL_INPUT_BYTES        = 256 KiB（沿用）
MAX_RAW_BODY_CHARS          = 12,000（沿用）
```

另设防御性 `MAX_LINE_BYTES` 与 `MAX_WORKER_STDOUT_BYTES`，值只影响异常 minified/恶意输入，错误统一转为
`too_large`，不得无界分配。

### 4.2 adapter registry

提供单一入口：

```text
normalize_permission_edit(agent, tool_name, tool_input, cwd)
  -> AdapterOutcome::Supported(PermissionEditIntent)
   | AdapterOutcome::UnsupportedNativeEdit(reason)
   | AdapterOutcome::NotNativeEdit
   | AdapterOutcome::Invalid(reason)
```

映射：

- Claude + `Edit` → `TextReplace`；严格检查 path/string/bool 类型与长度；
- Claude + `Write` → `WholeFileWrite`；
- Claude + `NotebookEdit` → `Unsupported`，不创建 read target；
- Codex + `apply_patch` → `PatchSet`；
- 其它 agent/tool → `NotNativeEdit`。

`permissions.rs::summarize_tool` 继续负责通用/IM raw body；在 `parse_permission` 内额外调用 adapter，
将结果放到 `ConfirmTask.popup_edit`。Vue 不接触原生 tool schema。

### 4.3 Codex patch parser

严格状态机解析：

1. 完整且唯一的 Begin/End envelope；
2. Add / Update / Delete File header；
3. Update 可选 Move to；
4. `@@` section、context / add / delete 行；
5. 路径不能为空、无 NUL，文件数与总输入受限；
6. 未知 header、重复非法操作、段未闭合、尾部垃圾均整次失败。

parser 只解释展示所需结构，不执行 patch。原 patch 永远保留在现有 raw body；parser 失败回到 raw params，
不能展示部分文件后把其余静默丢掉。

### 4.4 初始 Diff

- Edit：old/new 字符串经 `similar` 生成拟议 hunk，状态 `payload_only`；
- Write：生成“拟议完整内容”文件模型，before 标未知，不把统计标成真实新增；
- apply_patch：按 patch context / +/- 行生成多文件模型；
- Notebook/invalid：无 Diff model，状态 `unsupported`，只保留 raw body。

初始模型在短命 Hook 进程中只做 CPU 解析，不做任何 metadata/open/read/canonicalize。

### 4.5 P1 测试

在 `src-tauri/tests/fixtures/permission_diff/` 保存去敏后的真实 shape：

- Claude Edit：替换、删除、`replace_all`、Unicode、畸形字段；
- Claude Write：已有/新文件语义只测试载荷模型；
- Claude NotebookEdit：稳定落入 unsupported；
- Codex：单/多文件 Add、Update、Delete、Move、多个 hunk；
- Codex：未知 header、缺 envelope、超文件数、路径异常、尾部垃圾整次 fallback；
- 非原生 Bash/MCP/未知 Agent 不创建 edit intent。

同时锁定 hunk 行号、missing newline marker、add/delete 统计和 serde round-trip。

## 5. P2：Popup-only IPC

**触点**：`ipc/mod.rs`、`daemon/request.rs`、`app/mod.rs`、`commands.rs`、`lib/types.ts` 及相关测试。

### 5.1 wire 字段

- `ConfirmTask.popup_edit: Option<PermissionEditIntent>`；
- `ShowPayload.popup_edit: Option<PermissionEditIntent>`；
- `PopupInit.popup_edit: Option<PermissionEditIntent>`；
- `ConfirmSpec`、`ConfirmRequest`、`ConfirmEntry.request` 保持原 shape；
- `ConfirmEntry.show` 持有 popup edit intent，IM transport 只拿 `request`，不会看到该字段。

daemon `create_confirm` 在接受 intent 时验证：

- `agent_kind` 与 intent agent 相同；
- 当前仅允许 `claude` / `codex`；
- native tool 与 adapter 支持表匹配；
- workspace 与 task project 相同；
- 文件数、字符串与序列化总大小不越界；
- invalid intent 返回 confirm 创建错误，Hook 沿现有 fail-open 回 Agent 原生弹窗，不能 panic。

### 5.2 cold / warm Popup

- `AppState` 增 `popup_edit: Option<_>`，所有非 Popup 构造点显式填 `None`；
- cold helper 从初始 `ShowPayload` 写入 `AppState.popup_edit`；
- warm helper 继续以 `WarmPopup.show` 为当前真源，`popup_init` 每次领用时取对应 intent；
- `PopupInit` 返回 intent，但不返回任何 before 内容；
- warm popup 收到下一请求时，前端以 `ConfirmRequest.id` 重建状态并清掉旧 Diff。

### 5.3 P2 测试

- 新旧 payload 缺 `popupEdit` 均可反序列化；
- supported intent 从 `ConfirmTask` 到 `ShowPayload/PopupInit` 不丢字段；
- `ConfirmRequest` 的序列化快照与功能前一致，不出现 `popupEdit` / before；
- IM Confirm renderer 输入类型不变；
- mismatched agent/workspace/tool/oversized intent 被 daemon 拒绝；
- cold/warm `popup_init` 分别返回当前请求 intent，待命 warm 返回 `None`。

## 6. P3：快照 Worker 与安全策略

**触点**：新 `permission_diff/{safety,worker,build}.rs`、`cli/mod.rs`、`dev_instance.rs`、`commands.rs`。

### 6.1 隐藏角色

新增 `AskHuman __permission-diff-worker`：

- 在 `cli::dispatch` 早期分流，stdin 读一次有界 JSON，stdout 只写一个有界 JSON；
- 所有错误编码成稳定 enum，不向 stdout/stderr 打印路径内容或文件内容；
- `dev_instance::classify_command` 把该角色列为 `Skip`，避免 Popup 子进程按无关 cwd 重定向到错误实例；
- 父进程用 `current_exe()`、piped stdin/stdout、closed stderr 或受控 stderr 启动，不用 shell；
- `kill_on_drop(true)`，外层 `tokio::time::timeout(300ms)`；timeout 后显式 kill + wait 回收；
- 读取 stdout 时再施加 byte cap，畸形/超限视为 worker failure。

### 6.2 Popup 后端命令

新增 Tauri command `enrich_permission_diff(request_id)`：

1. 从 cold `AppState` 或 warm `WarmPopup.show` 找当前 request；
2. 校验 request id 与 Confirm request id 一致；
3. 对 read targets 先做无 I/O 的高风险路径分类；
4. 受保护 target 直接生成 per-file `protected_path`，只把剩余安全 targets 交给 Worker；
5. 若无安全 target，不 spawn；
6. 合并 Worker 结果与初始 Diff，返回带 request id 的最终模型。

命令不得持有 WarmPopup mutex 跨 await；先 clone 有界 intent 后释放锁，再 spawn。

### 6.3 macOS 预跳过

在 Popup Helper 内只用字符串/path component 判断，禁止预检时 `read_dir`、`metadata`、`canonicalize`：

- `$HOME/Desktop`、`Documents`、`Downloads`；
- `$HOME/Library/Mobile Documents`、`CloudStorage` 等 iCloud / File Provider 根；
- `/Volumes` 下网络盘/可移动卷；
- component 边界匹配，不能把 `DocumentsBackup` 误判成 `Documents`。

Linux 不应用 macOS TCC root 表，但仍执行通用路径/资源限制。路径判断写成纯函数并使用合成 home 测试，
测试本身不得访问真实受保护目录。

### 6.4 Worker 读取

对每个 target：

1. 以显式 workspace 解析相对路径并做词法 normalize；
2. 逐段使用 symlink-aware 检查，遇到不确定/循环/跳入受保护 root 即拒绝；
3. 最终文件 no-follow open；open 后 fstat 验证 regular file；
4. 在读取前检查 metadata size，流式读取时再次实施 1MiB / 4MiB 预算；
5. 严格 UTF-8 decode；
6. 读取/算法期间反复检查内部 deadline；
7. 不读取目录、device、socket、FIFO；不写任何文件。

文件不存在只返回 typed status，由 operation 语义决定 new file 或 source mismatch。

### 6.5 快照增强

- Edit：定位 old string；`replace_all` 展开所有 occurrence；补 context/行号；歧义或 mismatch 保留 initial；
- Write：before 存在时全文 diff，不存在时 empty → content；
- Patch Add：path 不存在为新文件；已存在则标 mismatch，但保留 patch；
- Patch Update：按 context 定位并补真实行号；不执行或写回 patch；
- Patch Delete：读取成功后显示删除内容；缺失标 mismatch；
- Patch Move：读取 old path，展示 old → new；new path 已存在时显示冲突状态；
- 每个文件独立失败，安全文件仍可增强；全局状态由 per-file 状态汇总。

hunk 统一带 3 行 context。构建完整 hunk 后再实施预算：每文件 400 行、全局 3000 行；不能从 hunk 中间
硬切。被省略的文件/hunk/行精确累计到模型。

### 6.6 P3 测试

- 纯函数：path normalize、component prefix、macOS root、`DocumentsBackup` 反例、cwd 外普通路径；
- tempdir：正常 UTF-8、缺失、新文件、非 UTF-8、1MiB 边界、超限、目录、FIFO/symlink（按平台 cfg）；
- 多文件：64/65、总计 4MiB 边界、部分 protected + 部分安全；
- Edit：唯一/歧义/missing/replace_all/Unicode/newline；
- Write：existing/new/unchanged/大改；
- Patch：Add/Update/Delete/Move 快照增强与 mismatch；
- 截断：400/3000 边界、完整 hunk、精确 omitted counters；
- Worker：正常 JSON、崩溃、畸形、stdout 超限、300ms timeout、kill + wait 无残留；
- request id mismatch、warm slot 变化、mutex 不跨 await。

timeout 测试通过可注入的 worker runner / test-only child fixture 制造阻塞，不依赖真实慢磁盘。

## 7. P4：Popup Diff UI

**触点**：`lib/types.ts`、`lib/ipc.ts`、`views/popup/usePopupCore.ts`、`ConfirmPane.vue`、
新 `PermissionDiffPane.vue` / `permissionDiff.ts`、`popup.css`、i18n messages。

### 7.1 前端状态

- `PopupInit`/domain TS 类型与 Rust camelCase 模型逐字段对齐；
- `usePopupCore` 按 `confirmRequest.id` 保存 `permissionDiff`、`permissionDiffLoading`；
- 有 initial Diff 时立即赋值；Notebook/unsupported 只显示状态 + raw；
- 在现有 popup ready / 首帧完成并触发窗口 show 后，fire-and-forget 调 `enrichPermissionDiff(id)`；
- enrichment promise 不得被 `await` 到上屏链路；
- 返回 id 不等于当前 request 时丢弃；关闭/被抢答后的错误静默处理，不弹 toast。

增加性能断言/埋点：`permission_diff.worker_start` 必须晚于本次 `fe.painted`，首帧指标不因 Worker 回归。

### 7.2 组件

`ConfirmPane.vue` 只编排：

- 有 permission edit 时挂载 `PermissionDiffPane`；
- 原 `detail.bodyMd` 移入默认关闭的 `<details>`“原始工具参数”；
- 无 edit intent 时保持当前 body 展示，避免影响 Bash/MCP/未知工具。

`PermissionDiffPane.vue` 负责：

- stats + 始终可见 snapshot 状态；成功时为轻量提示，失败或不完整时显示醒目警告；
- file header、Move path、hunks、old/new 行号；`cwd` 内路径相对显示，外部路径保持绝对路径，hover 显示原值；
- `context/add/delete/meta` class；
- truncated banner 和精确省略计数；
- loading 状态只更新 badge，不用骨架屏覆盖 initial Diff；
- 所有代码/路径用 `{{ text }}`，不使用 `v-html`。

CSS：

- 单栏、等宽字体、横向滚动；
- 红/绿背景同时保留 +/-；
- light/dark token 均满足可读性；
- 560px 默认窗和 420px 最小宽度下不挤压审批选项；
- Diff 区内部不抢占整个弹窗滚动，现有 focus/键盘选择保持可用。

### 7.3 i18n

新增 zh/en key：

- Diff 标题、原始参数、文件/行统计；
- 每种 `SnapshotStatus`；
- snapshot time；
- omitted files/hunks/lines；
- Notebook/unsupported 说明。

状态文案只由 enum + 参数格式化，前端不解析 Rust error string。

### 7.4 P4 测试

- `permissionDiff.ts`：状态/计数/i18n 参数纯函数；
- Vue：initial → enriched 替换、late result 丢弃、unsupported/raw-only；
- 多文件/Move/截断 DOM；
- 文件内容含 `<script>`、HTML entity、反引号时只作为文本；
- raw `<details>` 默认关闭，展开后沿用 sanitized Markdown；
- 无 edit intent 的 ConfirmPane DOM 与现状一致；
- 420px viewport 下行号、横向滚动、选项与 textarea 可用；
- keyboard submit、dismiss、拒绝原因输入不回归。

## 8. P5：端到端回归与文档收尾

### 8.1 自动验证

实现过程每个 P 完成先跑窄测试，最终统一运行：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
pnpm test
pnpm build
./scripts/install.sh
```

`./scripts/install.sh` 是功能逻辑变更后的强制步骤；安装成功后，后续人工确认必须使用新安装的 `AskHuman`。

### 8.2 真实权限弹窗矩阵

使用无敏感内容的临时目录逐项验收：

| 场景 | 预期 |
|---|---|
| Claude Edit existing file | 首屏拟议 hunk；随后补上下文/行号 |
| Claude Edit mismatch / replace_all | 状态准确；审批仍可用 |
| Claude Write existing / new | 完整 before/after 或 new file |
| Claude NotebookEdit | raw-only + unsupported 状态，不 spawn Worker |
| Codex multi-file Add/Update/Delete/Move | 文件分区、路径与统计正确 |
| 300ms timeout / >1MiB / non-UTF8 | 降级状态，无卡顿 |
| cwd 外普通临时文件 | 允许快照增强 |
| 合成/真实已知 TCC root | 不启动读取；不出现 AskHuman 文件授权提示 |
| warm / cold popup | 均正确，迟到结果不串请求 |
| IM 先答 / 本地先答 | 首答胜出不变，Worker 无残留 |
| Bash/MCP/unknown schema | 保持现有 raw detail，不显示伪 Diff |

真实 TCC 检查只做无害、可恢复路径，禁止为了测试读取用户真实 Desktop/Documents 内容；可用受控测试文件并由
用户观察系统提示。项目现有 `docs/PROGRESS.md` 中 TCC 真机待办保持独立，除非该次验证同时得到用户明确验收。

### 8.3 性能与数据泄漏检查

- 用现有 popup perf harness 比较功能前后：无 edit intent 与 edit intent 的 `fe.painted` 不出现文件 I/O 等待；
- 搜索 daemon/IM/history serde 和日志，确认无 before 内容字段或内容日志；
- Worker 正常/timeout/Popup close 后检查无遗留进程；
- 检查 release binary size 增量，`similar 2.7` 默认无额外依赖，记录但不设阻断阈值。

### 8.4 文档

- 实现完成后把 spec 状态更新为“已实现，待/已验收”；
- 在 `docs/plans/agent-permission-approval.md` 的 Confirm 渲染处增加本规格链接，不重写原计划历史；
- 仅当实现改变 repository-wide 架构地图或不变量时才改 `docs/overview.md`；按当前方案预计无需修改主 overview；
- 清理 `docs/PROGRESS.md` 本任务 section；保留其它既有待办。

## 9. 实现顺序与提交边界

```text
P1 model + adapters + payload diff
  → P2 popup-only IPC
    → P3 worker + safety + snapshot enhancement
      → P4 popup component + async lifecycle
        → P5 full verification + docs
```

建议提交边界：

1. `refactor(permissions): add native edit preview domain model`
2. `feat(popup): show native edit diffs in permission prompts`
3. `test(popup): cover permission diff fallbacks and worker limits`
4. `docs(permissions): document native edit diff behavior`

若实现期间必须改变 D1–D13、TCC 路径策略、Worker 预算、Notebook 范围或 Popup-only 数据边界，先更新规格并
通过 AskHuman 获得用户确认；不能在编码中自行扩 scope。

## 10. 完成定义

只有同时满足以下条件才可结束实现任务：

- spec §11 的十条验收标准全部有自动测试或明确人工记录；
- Rust tests、Vitest、frontend build、`./scripts/install.sh` 全部成功；
- 新安装 AskHuman 的 Claude/Codex 真实权限弹窗验收通过；
- TCC 已知路径未启动 Worker，Popup 首帧未被读盘阻塞；
- IM/历史/daemon 数据边界经测试与代码检查确认；
- 用户通过 AskHuman 明确确认可以结束且没有更多任务。
