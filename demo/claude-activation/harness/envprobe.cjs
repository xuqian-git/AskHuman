'use strict';

// CLI-tool environment probe (the "no-hook" path).
//
// 让 claude 在会话里用 Bash 工具跑：
//   node /…/demo/claude-activation/harness/envprobe.js
// 它模拟「AskHuman 被 Agent 当子进程调用」那一刻能看到什么：
//   - claude 注入的 env（CLAUDE_CODE_SESSION_ID / CLAUDECODE / …）
//   - 自身进程链，以及能否向上 walk 到 claude 进程拿到其 pid
// 然后把会话进程 pid 写入 logs/claude.pid.json（供 poller 守活），
// 并把全量 env（敏感打码）落到 logs 里。
//
// 与 hooklog 不同：这是工具调用，stdout 会回显给 claude / 用户，所以这里
// 故意打印一份可读摘要。

const C = require('./common.cjs');

function main() {
  const chain = C.processChain(process.pid);
  const { agent, candidates } = C.guessAgentPid(chain);
  const env = C.agentEnv();

  const report = {
    ts: C.nowIso(),
    epoch_ms: Date.now(),
    self_pid: process.pid,
    self_ppid: process.ppid,
    agent_pid: agent ? agent.pid : null,
    agent_comm: agent ? agent.comm : null,
    agent_command: agent ? agent.command : null,
    agent_token: agent ? agent.token : null,
    agent_alive: agent ? C.probeAlive(agent.pid) : 'unknown',
    claude_env: env,
    session_id_from_env: env.CLAUDE_CODE_SESSION_ID || null,
    chain,
    agent_candidates: candidates.map((c) => ({ pid: c.pid, comm: c.comm, token: c.token })),
    full_env_redacted: C.redactedEnv(),
  };

  C.appendJsonl('envprobe.jsonl', report);
  C.writeJson('envprobe-latest.json', report);

  if (agent && agent.pid) {
    C.writePidFile({
      pid: agent.pid,
      comm: agent.comm,
      command: agent.command,
      session_id: report.session_id_from_env,
      source: 'envprobe',
      ts: report.ts,
    });
  }

  // 可读摘要（stdout 会回显给 claude 与用户）
  const lines = [];
  lines.push('=== AskHuman ENV PROBE (no-hook path) ===');
  lines.push(`time            : ${report.ts}`);
  lines.push(`self pid/ppid   : ${report.self_pid} / ${report.self_ppid}`);
  lines.push('');
  lines.push('--- Claude-injected env (key question: is session id here?) ---');
  if (Object.keys(env).length === 0) {
    lines.push('(none found — no CLAUDE_*/CURSOR_*/CODEX_* env visible)');
  } else {
    for (const [k, v] of Object.entries(env)) lines.push(`${k} = ${v}`);
  }
  lines.push('');
  lines.push('--- agent process discovered by walking the tree ---');
  if (agent) {
    lines.push(`agent_pid       : ${agent.pid} (alive=${report.agent_alive}, matched "${agent.token}")`);
    lines.push(`agent_comm      : ${agent.comm}`);
    lines.push(`agent_command   : ${agent.command}`);
  } else {
    lines.push('(could not locate an agent process in the parent chain)');
  }
  lines.push('');
  lines.push('--- process chain (self -> ... -> root) ---');
  for (const e of chain) {
    lines.push(`  pid=${e.pid} ppid=${e.ppid} comm=${e.comm}`);
  }
  lines.push('');
  lines.push(`wrote: logs/envprobe-latest.json , logs/claude.pid.json`);
  process.stdout.write(lines.join('\n') + '\n');
}

try {
  main();
} catch (e) {
  process.stdout.write('envprobe error: ' + String(e && e.stack ? e.stack : e) + '\n');
}
process.exit(0);
