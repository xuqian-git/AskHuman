# Codex MCP AskHuman 顶层阻塞与取消清理

> 状态：已实现并完成自动化验证；新会话生效（2026-07-11）  
> 范围：Codex MCP 自动集成；不包含 PermissionRequest Hook 或权限弹窗

## 1. 目标与源码结论

Codex Code Mode 的 `functions.exec` / `functions.wait` 默认每 10 秒 yield。yield 不会取消后台 cell 或 MCP
调用，但会把控制权交回模型；模型随后可以不再等待或显式 terminate。因此仅配置 24 小时
`tool_timeout_sec`，不能保证 agent 始终阻塞在 AskHuman 提问上。

Codex 原生支持：

```toml
[features.code_mode]
direct_only_tool_namespaces = ["mcp__askhuman"]
```

配置后 AskHuman namespace 成为 `DirectModelOnly`：在 CodeModeOnly 下仍暴露为顶层工具，但不会进入
`functions.exec` 的嵌套工具集。顶层 MCP 调用返回前不会产生下一次模型采样；正常情况下只会因人类作答、
现有 24 小时 tool timeout、用户显式中断、session/process 退出而结束。外部强杀进程不在可保证范围内。

源码证据：

- 默认 yield：Codex `codex-rs/code-mode-protocol/src/runtime.rs:11-12`；
- yield 只发送事件：`codex-rs/code-mode/src/cell_actor/mod.rs:236-252`；
- 后台 cell 跨 turn 继续：`codex-rs/core/tests/suite/code_mode.rs:2423-2495`；
- direct-only exposure 与嵌套排除：`codex-rs/core/src/tools/spec_plan.rs:204-224,430-469`；
- direct-only 回归测试：`codex-rs/core/src/tools/spec_plan_tests.rs:1077-1112`；
- MCP timeout 调用链：`codex-rs/codex-mcp/src/connection_manager.rs:760-779`、
  `codex-rs/rmcp-client/src/rmcp_client.rs:1070-1092`。

## 2. 产品与配置边界

- 这是 Codex **MCP 配置产物**的一部分，不新增开关，不属于 Hook 产物；
- 新安装/切换到 Codex MCP 时自动写入；
- 已安装 Codex MCP 但缺 direct-only 时，现有 MCP 产物显示“需更新 / 更新”，不静默迁移；
- Codex CLI、Claude/Cursor/Grok MCP 不写该字段；
- 更新后提示重启 Codex 或新开 session；
- 不修改 Codex 源码。

## 3. 最小编辑与所有权

1. 字段不存在时创建数组并追加 `mcp__askhuman`；数组存在时只在缺失时追加；
2. 保留其它 namespace、原顺序、注释、未知键、数组键和父表；类型异常时中止、不覆盖；
3. 安装前已有同名项时不改、不认领，卸载必须保留；
4. 仅当 AskHuman 实际追加时，在自有 `~/.askhuman/integration-state.json` 原子记录
   `codex.direct_only_namespace_added_by_askhuman = true`；该文件不是用户设置模型；
5. 卸载仅在所有权为 true 时移除该数组项，然后清除所有权；不删其它项、数组键或父表；
6. sidecar 缺失、损坏或状态不明时保守保留，宁可留下无害项也不误删；
7. Codex 配置写成功而 sidecar 写失败时回滚；卸载失败保持可重试，不伪报成功；
8. 配置和 sidecar 均使用原子写，sidecar 在 Unix 上限制为 owner-only。

## 4. MCP 取消清理

当前 `mcp/ask.rs` 使用 `spawn_blocking + std::process::Command::output()`。MCP handler 被取消时，blocking
task/子进程可能继续运行，留下“问题仍显示、结果无人接收”的孤儿请求。

- 改为 cancellation-aware 的 `tokio::process::Command`，启用 `kill_on_drop(true)`；
- 保持 stdin 为空、stdout/stderr 捕获和 `ASKHUMAN_FROM_MCP=1`；
- 验证子进程断连会让 daemon 取消在途请求，并使 popup/IM 正确收尾；若当前 IPC 不保证，则让 handler
  直接持有可取消 daemon request handle；
- 取消是基础设施/调用方终止，不得伪装成人类 cancel/deny，也不得生成虚构答案。

## 5. 实施步骤

1. 为 `integration-state.json` 增加路径、向后兼容读取、原子保存和权限测试；
2. 扩展 Codex TOML 纯函数：检查/追加/按所有权移除 direct-only；
3. 将 direct-only 缺失纳入 `mcp_config::needs_update(Codex)`，接通现有 MCP 更新按钮；
4. 组合安装/卸载事务与失败回滚；其它 Agent 路径保持不变；
5. 改造 MCP ask 子进程和取消路径；
6. 单测与集成测试后运行格式化、Rust 测试、`./scripts/install.sh`；
7. 用新安装的 AskHuman 验收 Codex direct-only 顶层阻塞、正常回答和显式取消收尾；
8. 更新 `docs/overview.md`，清理 `docs/PROGRESS.md`，再进入独立权限弹窗计划。

## 6. 测试矩阵

| 场景 | 预期 |
|---|---|
| 新安装 Codex MCP | server、24h timeout、direct-only 一次写齐 |
| 已安装但缺 direct-only | MCP 产物显示需更新；点击后追加 |
| 数组已有其它 namespace | 只追加 AskHuman 项，其它值/顺序/注释保持 |
| 安装前已有同名项 | 不重复、不认领；卸载保留 |
| AskHuman 添加同名项 | sidecar 记所有权；卸载只删该项 |
| sidecar 缺失/损坏 | 保守保留，不猜测所有权 |
| 字段类型异常 | 安装失败，不覆盖原文件 |
| sidecar 保存失败 | 回滚 Codex 配置，不报告成功 |
| 重复安装/更新/卸载 | 幂等，不重复数组项，不破坏用户配置 |
| 其它 Agent MCP | 输出与现有行为逐字节一致 |
| 正常 Codex 提问 | 只能顶层调用，持续阻塞到结果/24h timeout/外部取消 |
| MCP 请求显式取消 | 子进程、daemon 请求、popup/IM 收尾，无孤儿问题 |

## 7. 提交建议

1. `fix(codex): keep AskHuman MCP calls blocking`
2. `fix(mcp): clean up cancelled AskHuman requests`
3. `docs(codex): document blocking MCP integration`

## 8. 本次验证记录（2026-07-11）

- `cargo test`：526 passed、0 failed、1 ignored；
- `./scripts/install.sh`：前端 typecheck/build、release 编译、签名和安装成功；
- 新安装的 `AskHuman doctor --json`：更新前 Codex MCP `needsUpdate=true`，执行现有
  `agents update codex --mcp` 后为 false；
- 实际 `~/.codex/config.toml` diff：保留其它配置，只新增 `[features.code_mode]` direct-only；既有
  AskHuman timeout 从 Codex 归一化的 `30.0/86400.0` 写回本产物模板的 `30/86400`；
- `~/.askhuman/integration-state.json` 正确记录所有权，Unix 权限为 0600；
- `codex --strict-config doctor --json`：`config.load` 为 ok，`config.toml parse` 为 ok；
- 取消单测确认 drop MCP handler 的 output future 会终止子进程；daemon 既有 `wait_cli_eof` 路径会取消
  整个请求并收尾 popup/IM；
- 当前会话的 tool plan 不热重载，顶层 direct-only 可见性需重启 Codex 或新开 session 后验收。
