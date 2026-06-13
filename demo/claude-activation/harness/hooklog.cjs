'use strict';

// Claude Code lifecycle-hook logger.
//
// 由项目级 .claude/settings.json 的各 hook 调用：
//   node hooklog.js <EventName>
// 它从 stdin 读 hook JSON，补上墙钟时间 / 自身 pid·ppid / 进程链 / 猜到的
// claude 进程 pid / 关键 env，追加一行到 logs/events.jsonl。
//
// 关键纪律（fail-open）：无论如何都 exit 0、stdout 不输出任何东西。
//   - UserPromptSubmit / SessionStart 的 stdout 会被当作「注入上下文」喂给模型，
//     所以这里绝不能往 stdout 写日志，否则会污染会话。

const fs = require('fs');
const C = require('./common.cjs');

function readStdin() {
  try {
    return fs.readFileSync(0, 'utf8');
  } catch {
    return '';
  }
}

function main() {
  const event = process.argv[2] || 'Unknown';
  const raw = readStdin();

  let hook = null;
  try {
    hook = raw ? JSON.parse(raw) : null;
  } catch {
    hook = { _parse_error: true, _raw: raw.slice(0, 2000) };
  }

  const chain = C.processChain(process.pid);
  const { agent, candidates } = C.guessAgentPid(chain);

  const rec = {
    ts: C.nowIso(),
    epoch_ms: Date.now(),
    event,
    // hook JSON 里的关键字段（best-effort）
    json_event: hook && hook.hook_event_name,
    session_id: hook && hook.session_id,
    transcript_path: hook && hook.transcript_path,
    cwd: hook && hook.cwd,
    permission_mode: hook && hook.permission_mode,
    source: hook && hook.source, // SessionStart
    reason: hook && hook.reason, // SessionEnd
    prompt: hook && typeof hook.prompt === 'string' ? hook.prompt.slice(0, 200) : undefined,
    tool_name: hook && hook.tool_name, // Pre/PostToolUse
    stop_hook_active: hook && hook.stop_hook_active, // Stop
    // 进程视角
    hook_pid: process.pid,
    hook_ppid: process.ppid,
    agent_pid: agent ? agent.pid : null,
    agent_comm: agent ? agent.comm : null,
    agent_token: agent ? agent.token : null,
    // env 里 claude 注入的关键变量（hook 子进程也会拿到 CLAUDE_CODE_SESSION_ID）
    env: C.agentEnv(),
    // 完整进程链（便于核对 agent_pid 猜得对不对）
    chain,
    // 多个候选（理论上应只有一个 agent）
    agent_candidates: candidates.map((c) => ({ pid: c.pid, comm: c.comm, token: c.token })),
  };

  C.appendJsonl('events.jsonl', rec);

  // 把「当前会话的 claude 进程」写入 pid 文件，供 poller 守活。
  // 仅在猜到 agent 时更新；SessionEnd 也照常写（poller 自己判存活）。
  if (agent && agent.pid) {
    C.writePidFile({
      pid: agent.pid,
      comm: agent.comm,
      command: agent.command,
      session_id: rec.session_id || (rec.env && rec.env.CLAUDE_CODE_SESSION_ID) || null,
      source: `hook:${event}`,
      ts: rec.ts,
    });
  }
}

try {
  main();
} catch (e) {
  // 即使内部出错也要 fail-open，并留个痕迹。
  try {
    C.appendJsonl('events.jsonl', {
      ts: C.nowIso(),
      event: process.argv[2] || 'Unknown',
      _error: String(e && e.stack ? e.stack : e),
    });
  } catch {}
}
process.exit(0);
