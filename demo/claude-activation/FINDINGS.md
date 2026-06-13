# Claude Code 激活信号 Demo —— 调研结论与实测记录

> 本文档专门记录 IM 渠道激活方案（`docs/todos/im-channel-activation.md`）相关的**方案/调研结论**与**实测结果**。
>
> 关联：设计讨论见 `docs/todos/im-channel-activation.md`（三层信号模型：进程存活=电平骨干、turn-start↔turn-end=判忙闲、TTL=仅兜底）。本 Demo 用来**实测验证**那套模型对 Claude Code 是否成立。

---

## 0. 红线：未经许可，不得实际调用任何 Agent 做实测

**实际运行 claude / cursor-agent / codex 会消耗用户的 token。** 因此：

- **任何需要真正启动 / 提示 / 驱动 Agent 的实测，必须先经用户明确许可**（通过 `AskHuman` 征询），并由**用户来操作 Agent**（启动、发提示、停止、关窗口等）。AI 只负责搭 harness、观察日志、分析结果。
- 不得为了"顺便验证"而擅自 `claude -p "..."` / `codex exec ...` / `cursor-agent ...` 之类调用。
- 纯文档查证、读源码、跑 harness 自带的**非 Agent 冒烟自测**（直接 `node envprobe.cjs` 等）不受此限。

本轮范围：**只实测 Claude Code**。Cursor / Codex 仅停留在下方"文档结论"，要实测需另行征得许可。

---

## 1. 调研结论（来源＝各家官方文档，**截至落档尚未实测**）

> 下面结论来自 Claude Code / Cursor / Codex 官方文档查证（2026-06），**不是**实跑得到的。实测结果见 §5。

### 1.1 你最关心的问题：不用 Hook 能否拿到会话信息？

**Claude Code：能。** claude 调用 CLI 工具（Bash 工具子进程）时会向子进程注入环境变量：

| 变量 | 含义 |
|---|---|
| `CLAUDECODE=1` | 标识当前在 Claude 的子进程内 |
| `CLAUDE_CODE_SESSION_ID` | **会话 ID**；与 hook JSON 的 `session_id` 一致，`/clear` 时更新 |
| `CLAUDE_CODE_CHILD_SESSION=1` | v2.1.172+；仅 Claude 亲自 spawn 时设，可靠区分"嵌套会话"（IDE 集成终端里手开的 claude 不算） |
| `CLAUDE_PROJECT_DIR` | 项目根目录 |

→ 所以 **AskHuman 被 claude 当子进程调用时，自己读 env 就能拿到会话 ID，不必装 Hook。**

**但 env 有两个局限**（决定了 Hook 仍有不可替代的价值）：
1. env 只在"被调用那一刻"给你这些值——它**给不了"turn 开始/结束"事件**。要在"第一次提问之前"就 arm（好让用户更早发 here），仍需 turn-start Hook（`UserPromptSubmit`）。
2. env **不直接给 claude 进程 PID**——做"进程存活轮询"（电平骨干）需要 PID，得顺进程树向上 walk 找到 claude 进程。本 Demo 正是要验证这个 walk 是否可靠。

⚠️ 跨平台坑（macOS 不受影响）：Linux 上 `CLAUDE_CODE_ENV_SCRUB` 会把 Bash 子进程放进**隔离 PID namespace**，导致 `ps`/`pgrep`/`kill` 看不到宿主进程 → walk 进程树 / `kill -0` 失效。本机是 macOS，无此问题；但生产实现需注意 Linux 分支。

### 1.2 三家对照（均为文档结论，未实测）

| 维度 | Claude Code (claude 2.1.176) | Cursor Agent (cursor-agent CLI) | Codex CLI (0.135.0) |
|---|---|---|---|
| 子进程 env 带会话 ID？ | **是** `CLAUDE_CODE_SESSION_ID`（+`CLAUDECODE`/`CLAUDE_CODE_CHILD_SESSION`/`CLAUDE_PROJECT_DIR`） | **是（实测旁证，与文档相反）** `CURSOR_CONVERSATION_ID`（+`CURSOR_AGENT=1`/`CURSOR_INVOKED_AS=agent`），见 §1.5 | **否（文档未见）**：subprocess env 受 `[shell_environment_policy]` 控制，默认还剥离名字含 KEY/SECRET/TOKEN 的变量 |
| 不用 Hook 拿会话信息？ | **能** | **能拿 conversation_id**（读 `CURSOR_CONVERSATION_ID`），见 §1.5 | 基本不能 |
| turn-start 事件 | `UserPromptSubmit` | `beforeSubmitPrompt` | `UserPromptSubmit`（需 Codex hooks） |
| turn-end 事件 | `Stop` | `stop` | `Stop`（Codex hooks）/ 稳定的 `notify` 仅 `agent-turn-complete` |
| 会话结束事件 | `SessionEnd`（可靠） | `sessionEnd` | **无 SessionEnd** → 只能靠进程存活轮询兜底 |
| Hook 配置位置 | `~/.claude/settings.json` 或项目 `.claude/settings.json` | `~/.cursor/hooks.json` 或项目 `.cursor/hooks.json`（version 1） | `~/.codex/config.toml` 的 `[hooks]` 或 `hooks.json` |
| Hook 输入会话字段 | `session_id`/`transcript_path`/`cwd`/`permission_mode` | `conversation_id`/`generation_id`/`workspace_roots`/`transcript_path` | 同 CC 风格（事件名 `UserPromptSubmit`/`Stop`/…） |
| 进程粒度 | 单 `claude` 进程＝单会话 ✓ | **CLI 版应为单进程/会话**（设计 doc 里"Cursor=整个 IDE 粗粒度"指的是 IDE 版）⚠️待实测 | 单 `codex` 进程＝单会话 ✓ |
| 其它坑 | macOS 无 PID-namespace 问题 | 已知 bug：`AskQuestion` 工具不触发 pre/postToolUse；但 **Shell 调用照常触发**，我们用 Shell 调 AskHuman 不受影响 | hooks 需"信任"项目（见下）；env 剥离策略可能影响读 env |

### 1.3 Codex hooks 的"信任"机制（待看源码确认）

- 文档示例里出现过 `[features] codex_hooks = true`。**用户反馈：新版应已不需要这个 feature 标志位**，而是需要一次**"信任"校验**。
- 用户给出的线索：信任校验大致是 **`hash(路径 + hook 内容)`** 的形式——首次见到某 hook 配置要确认信任，内容变了要重新信任。
- **TODO（待用户提供 codex 源码后核对）**：确认信任值的确切计算方式（哪些字段参与哈希、存哪、`--dangerously-bypass-hook-trust` 的作用），以便将来生产里"自动安装 Codex hook"时能正确处理信任，不让用户每次手动确认。

### 1.5 实测旁证：cursor-agent 的 ambient env（零成本，非主动调用）

搭 harness 时直接 `node envprobe.cjs`（当前会话本就跑在 cursor-agent CLI 里），读到的**自身 ambient env** 里有：

```
CURSOR_INVOKED_AS = agent
CURSOR_AGENT = 1
CURSOR_CONVERSATION_ID = <uuid>
CURSOR_ASKPASS_SOCKET / CURSOR_ASKPASS_SECRET / CURSOR_RIPGREP_PATH
```

说明（这只是读自己进程的 env，**没有主动调用 cursor-agent 做测试**，不违反 §0）：
- **cursor-agent CLI 确实向 shell 子进程注入 `CURSOR_CONVERSATION_ID`**（=会话 ID）。这与 §1.2 之前依据文档得到的"Cursor 无 conversation_id env"结论**相反**——以此实测为准（至少对 cursor-agent CLI 2026.06 版本）。
- 所以将来若做 Cursor 支持，"无 Hook 拿会话 ID"对 **cursor-agent CLI** 也成立（IDE 版未测）。
- **进程识别注意**：cursor-agent 的可执行名是 `agent`（`~/.local/bin/agent … index.js`），**不含 "cursor-agent" 字样**。本 harness 默认按 `claude`/`codex`/`cursor-agent` 子串识别 agent 进程，对 cursor 会漏识别——将来测 Cursor 需把 `agent`/`index.js` 加入识别规则（现在 Claude 不受影响）。

### 1.6 关键结论（综合）

1. **"不用 Hook 拿会话 ID"：Claude（`CLAUDE_CODE_SESSION_ID`）与 cursor-agent CLI（`CURSOR_CONVERSATION_ID`）都成立**；Codex 文档未见对应 env（待实测）。
2. **三家都需 Hook 才能拿 turn-start 事件**（在第一次提问之前就 arm）；纯 env / notify 都给不了 turn-start。
3. **"会话是否还在"最终三家都得靠进程存活轮询兜底**（Codex 无 SessionEnd、Cursor 事件历史上不稳）——正是设计 doc §5 的电平骨干。
4. harness 自带的**非 Agent 冒烟自测**已通过：`poller` 能正确 `arm→LIVE→DEAD`（杀掉假进程即抓到死亡），证明电平信号链路 OK；`hooklog`/`envprobe` 读 env、回溯进程树、写日志均正常。

---

## 2. 本 Demo 要实测验证的清单

把上面的文档结论逐条用真机验证（只针对 Claude Code）。**实测已全部通过**（2026-06-13，claude 2.1.176，macOS arm64）：

- [x] **C1** ✓　claude 调 Bash 工具时，子进程 env 含 `CLAUDECODE=1` / `CLAUDE_CODE_SESSION_ID` / `CLAUDE_CODE_CHILD_SESSION=1` / `CLAUDE_CODE_ENTRYPOINT=cli`。**注意**：Bash 工具子进程**没有** `CLAUDE_PROJECT_DIR`；而 **hook 子进程有** `CLAUDE_PROJECT_DIR`（两类子进程 env 不完全一样）。
- [x] **C2** ✓　Bash 子进程 env 的 `CLAUDE_CODE_SESSION_ID` == hook JSON 的 `session_id` == hook env 的 `CLAUDE_CODE_SESSION_ID`，三者完全一致。
- [x] **C3** ✓　从 CLI 子进程向上 walk 能稳定定位 claude：`node → /bin/zsh(Bash工具包装) → claude → -zsh(登录shell) → login → Terminal`；claude 以 `claude` 名启动时 `comm` 就是 `claude`（不是版本化路径）。
- [x] **C4** ✓　turn-start(`UserPromptSubmit`)↔turn-end(`Stop`) 成对；中间夹 `PreToolUse`/`PostToolUse`。
- [x] **C5** ✓　见 §5 矩阵：**只有 `kill -9` 丢了 `SessionEnd`，进程存活轮询全程不漏**。
- [x] **C6** ✓　`/clear` 会 `SessionEnd(reason=clear)`→`SessionStart(source=clear)`，**session_id 轮换**（旧→新）但**进程 pid 不变** → 绑进程比绑 session_id 更稳。
- [x] **C7** ✓　项目级 `.claude/settings.json` 的 9 个 hook 全部被加载并触发（在子目录启动 claude 即生效，无需放到 git 根）。

---

## 3. Demo 组成

```
demo/claude-activation/
  .claude/settings.json   项目级 hooks：把 9 类生命周期事件都转给 hooklog.cjs
  harness/
    common.cjs             公共：进程树回溯 / 猜 agent pid / env 收集 / pid 文件 / kill -0 探活
    hooklog.cjs            被各 hook 调用：读 stdin 的 hook JSON + 补进程/env 信息 → logs/events.jsonl
    envprobe.cjs           "无 Hook 路径"探针：让 claude 用 Bash 跑它，dump env+进程树 → logs/envprobe-*.json
    poller.cjs             "电平骨干"：周期 kill -0 守活 logs/claude.pid.json 里的会话进程 → logs/poller.jsonl
  logs/                   运行时产物（已 .gitignore）
  FINDINGS.md             本文件
```

要点：
- harness 三件套都会把"猜到的 claude 进程 pid"写入 `logs/claude.pid.json`，poller 据此守活。hooklog 在第一次事件就写（hook 路径 arm），envprobe 在被调用时写（无 hook 路径 arm）。
- hooklog **绝不往 stdout 写**（`UserPromptSubmit`/`SessionStart` 的 stdout 会被当上下文注入模型），所有信息进日志文件；始终 `exit 0` fail-open。
- 路径在 `.claude/settings.json` 里写成**绝对路径**（仓库整体是一个 git repo，避免 `CLAUDE_PROJECT_DIR` 被算到 git 根导致 hook 找不到脚本）。若仓库迁移需同步改这里的绝对路径。

---

## 4. 运行方式

> 启动 / 操作 claude 由**用户**来做（见 §0 红线）。AI 负责起 poller、观察日志。

1. **（AI）起轮询器**（后台），它会等 `logs/claude.pid.json` 出现：
   ```bash
   node demo/claude-activation/harness/poller.cjs 1000
   ```
2. **（用户）在 demo 目录启动 claude**：
   ```bash
   cd demo/claude-activation && claude
   ```
   启动后用 `/hooks` 确认能看到本 demo 的 9 个 hook（source=Project）→ 验证 C7。
3. **（用户）按测试矩阵 §5 逐项操作**；每步 AI 读 `logs/events.jsonl` / `logs/poller.jsonl` / `logs/envprobe-latest.json` 分析。
4. 看日志的便捷命令：
   ```bash
   tail -f demo/claude-activation/logs/events.jsonl
   # 只看关键生命周期事件：
   node -e 'require("fs").readFileSync("demo/claude-activation/logs/events.jsonl","utf8").trim().split("\n").forEach(l=>{const r=JSON.parse(l);console.log(r.ts,r.event,"sid="+(r.session_id||"-"),"agent_pid="+r.agent_pid)})'
   ```

清理一次实测：`rm -f demo/claude-activation/logs/*.jsonl demo/claude-activation/logs/*.json`

---

## 5. 测试矩阵与实测结果

实测时间 2026-06-13，claude 2.1.176 / macOS arm64。一个 claude 会话＝一个独立 `claude` 进程。

### 5.1 turn / env / 进程定位（Phase A，1 个会话、2 个 prompt）

| # | 操作 | 实测结果 | 结论 |
|---|---|---|---|
| T1 启动+`/hooks` | 在 demo 目录起 claude | `SessionStart(source=startup)` 触发，agent_pid 立即写入、poller 在**任何 prompt 之前**就 armed | 项目级 hooks 生效；**光启动就能 arm** |
| T2 跑 envprobe | 让 claude Bash 跑探针 | env=`{CLAUDECODE,CLAUDE_CODE_SESSION_ID,CLAUDE_CODE_CHILD_SESSION,CLAUDE_CODE_ENTRYPOINT}`；walk 到 claude pid | **不用 Hook、读 env 就拿到会话 ID** |
| T3 干点活 | 两个普通 turn | 均 `UserPromptSubmit→PreToolUse→PostToolUse→Stop`，session_id 全程一致 | turn-start↔turn-end 成对可靠 |

### 5.2 会话结束 / 关闭矩阵（Phase B/C/D，**0 计费轮次**：仅靠启动 + 斜杠命令 + 外部 kill/关窗）

| 场景 | `SessionEnd`? | reason | session_id | 进程 | poller |
|---|---|---|---|---|---|
| `/clear` | **触发** + 紧接 `SessionStart(source=clear)` | `clear` | **轮换**（旧→新） | **不变**（同 pid） | 仍 LIVE |
| 正常 `/exit` | **触发** | `prompt_input_exit` | — | 退出 | **DEAD**（~0.9s 后） |
| **`kill -9`** | **不触发（事件丢失）** | — | — | 被杀 | **DEAD** ✓ |
| 直接关终端窗口 | **触发**（claude 收 SIGHUP 优雅收尾） | `other` | — | 退出 | **DEAD** |

poller 全程自动在 3 个会话间 re-arm（8079→10339→11073），每次 `arm→LIVE→DEAD` 正确。

### 5.3 实测结论（对照设计 doc §4.8 / §5）

1. **「电平骨干＝进程存活」被证实是唯一不漏的信号**：`kill -9`（模拟崩溃/强杀）下 `SessionEnd` **完全丢失**，只有进程存活轮询抓到了死亡。印证 §4.8「事件是边沿信号会丢、电平信号不漏」。
2. **关窗口 ≠ 崩溃**：直接关终端窗口时 claude 收到 SIGHUP 仍**优雅触发** `SessionEnd(reason=other)`；真正会丢事件的是 `kill -9` / 进程崩溃。
3. **绑「进程」比绑「session_id」稳**：`/clear` 会让 session_id 轮换但进程不变。若用 session_id 当会话身份，`/clear` 后会被误判成「新会话」；用 **claude 进程 pid 作电平骨干**则连续不断。
4. **不用 Hook 也能拿会话 ID（Claude）**：AskHuman 被 claude 当子进程调用时读 `CLAUDE_CODE_SESSION_ID` 即可；但仍需 Hook 才能在「第一次提问前」就 arm（`SessionStart`/`UserPromptSubmit`），以及拿 turn-start 以判「人是否回到电脑前」。
5. **低轮次测试法**（本次提炼）：生命周期类信号（SessionStart/SessionEnd/进程死亡/`/clear` 轮换）**全部可用 0 个 prompt 验证** —— 仅靠「启动 claude + 斜杠命令 + 外部 kill/关窗」。唯一需要真 prompt 的是 turn-start↔turn-end 成对（一次即可）。这对按轮次计费的 Agent（如 Cursor）很关键。

### 5.4 对设计 doc 的影响（建议回写 `docs/todos/im-channel-activation.md`）

- §6 表「Claude Code」一行可标注：**实测确认** 事件齐全 + `SessionEnd` 可靠（除 `kill -9`）；env 直带 `CLAUDE_CODE_SESSION_ID`（不用 Hook 即可拿会话 ID）。
- §10「PPID-at-ask 兜底」：实测 walk 路径为 `子进程 → /bin/zsh(Bash包装) → claude`，确认「向上 walk 找稳定 Agent 进程」可行且必要（直接 PPID 是临时 zsh）。
- 新增注意点：会话身份**应以进程 pid 为准**，因为 `session_id` 会随 `/clear` 轮换。

---

## 6. 低轮次（省 token）测试方法论

> 背景：有的 Agent **按轮次（turn）计费**——每发一次 prompt 收一次费（Cursor 尤其明显）。所以测试要把「信号验证」和「花钱的 turn」**解耦**：能用免费动作触发的信号，绝不发 prompt。

### 6.1 核心原则

1. **区分「免费动作」与「计费动作」**：
   - **免费**：启动 Agent 会话、斜杠命令（`/clear`、`/exit` 等不走模型）、外部 `kill`/关窗口、读自身进程的 ambient env、跑常驻 hook/poller。
   - **计费**：发一个 prompt（= 一个 turn）。
2. **把观测前移到免费动作上**：常驻 **hook 日志** + **进程存活轮询** + **ambient env 读取**，让大多数信号在「启动 / 关闭 / 斜杠命令」时就被记录，不需要对话。
3. **唯一要花钱的 turn 设计成「一次覆盖多个信号」**：用一个 prompt 同时验证 env 探针 + 工具调用 + turn 成对。

### 6.2 各信号需要几个 prompt（Claude 实测归纳）

| 要验证的信号 | 触发方式 | 计费 prompt 数 |
|---|---|---|
| 项目级 hooks 是否加载 / `SessionStart` / 首次 arm | 启动 claude 即触发 | **0** |
| hook 子进程能拿到哪些 env（含 `CLAUDE_CODE_SESSION_ID`） | `SessionStart` hook 自动记录 | **0** |
| `SessionEnd` 是否触发 + reason（正常 `/exit`） | 斜杠命令 `/exit` | **0** |
| `SessionEnd` 在崩溃下是否丢 / 进程存活轮询是否兜住 | 外部 `kill -9` | **0** |
| 关窗口的收尾行为 | 直接关终端窗口 | **0** |
| `/clear` 是否轮换 session_id / 进程是否不变 | 斜杠命令 `/clear` | **0** |
| **Bash 工具子进程**的 env（区别于 hook 子进程） | 让 claude 跑一次 envprobe（Bash 工具） | **1** |
| turn-start↔turn-end 成对（`UserPromptSubmit`→…→`Stop`） | 发一个会调用工具的 prompt | **1**（可与上一行**合并**） |

→ **整套 Claude 验证的理论最小成本 = 1 个 prompt**：让 claude 用 Bash 跑 envprobe（同时覆盖「Bash 子进程 env」+「turn 成对」+「PreToolUse/PostToolUse」）；其余全部 0 prompt。本次实测实际只花了 **2 个 prompt**（envprobe + 一次读文件），关闭矩阵 B/C/D 全程 **0 prompt**。

### 6.3 套到其它 Agent（按轮次计费的 Cursor 最该用）

同一思路可平移（待实测，需许可）：
- **Cursor**：`sessionStart`/`sessionEnd`/`stop` 都能靠「启动 cursor-agent + 关闭/外部 kill」触发（0 轮次）；`CURSOR_CONVERSATION_ID` 可直接读 ambient env（0 轮次，见 §1.5）；只有 `beforeSubmitPrompt`↔`stop` 成对需 1 轮。
- **Codex**：`SessionStart`/进程死亡同理 0 轮次；`notify`(agent-turn-complete) / `Stop` 需 1 轮触发一次 turn。
- 通用：先把 hook 日志 + poller 挂上，再用**一个**精心设计的 prompt 收集所有「必须对话才有」的信号。
