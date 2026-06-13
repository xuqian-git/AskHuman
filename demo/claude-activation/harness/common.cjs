'use strict';

// Shared helpers for the Claude Code activation-signal demo.
// Pure Node (no deps). Used by hooklog.js / envprobe.js / poller.js.

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

// harness/ 的上一级即 demo 根目录；脚本自定位，不写死绝对路径。
const DEMO_ROOT = path.dirname(__dirname);
const LOGS_DIR = path.join(DEMO_ROOT, 'logs');
const PID_FILE = path.join(LOGS_DIR, 'claude.pid.json');

// harness 自身进程的标记：用于在进程树里把「我们自己」排除掉，
// 否则 demo 目录名本身含 "claude" 会误判。用扩展名无关的基名。
const SELF_MARKERS = ['hooklog', 'envprobe', 'poller'];

// 把这些 token 当作「Agent 会话进程」的识别依据（子串匹配，大小写不敏感）。
// 用子串是因为 claude 是指向版本化二进制的符号链接，进程名可能是
// /Users/.../share/claude/versions/2.1.176 这种解析后路径。
const AGENT_TOKENS = ['claude', 'codex', 'cursor-agent'];

function nowIso() {
  return new Date().toISOString();
}

function ensureLogs() {
  fs.mkdirSync(LOGS_DIR, { recursive: true });
}

function ps(pid, fmt) {
  try {
    return execFileSync('ps', ['-o', fmt, '-p', String(pid)], {
      encoding: 'utf8',
    }).trim();
  } catch {
    return '';
  }
}

function basename(p) {
  if (!p) return '';
  return String(p).split('/').pop();
}

// 从 startPid 向上回溯进程树，直到 pid<=1 或出现环。
// 每个节点含 { pid, ppid, comm(可执行路径/名), command(完整命令行) }。
function processChain(startPid) {
  const chain = [];
  const seen = new Set();
  let pid = Number(startPid);
  while (pid && pid > 1 && !seen.has(pid)) {
    seen.add(pid);
    const ppidComm = ps(pid, 'ppid=,comm=');
    if (!ppidComm) break;
    const m = ppidComm.match(/^\s*(\d+)\s+(.*)$/);
    const ppid = m ? Number(m[1]) : 0;
    const comm = m ? m[2].trim() : '';
    const command = ps(pid, 'command=');
    chain.push({ pid, ppid, comm, command });
    pid = ppid;
  }
  return chain;
}

function isSelf(entry) {
  const hay = `${entry.comm || ''} ${entry.command || ''}`;
  return SELF_MARKERS.some((mk) => hay.includes(mk));
}

// 匹配「Agent 进程」：主要看可执行路径 comm（如 claude 是
// /Users/.../share/claude/versions/2.1.176，含 "claude"），辅以 argv0 basename
// 精确匹配。**不**对完整命令行做子串匹配——否则命令里出现的路径
// （如 cd .../demo/claude-activation）会把无辜的 shell 误判成 agent。
function matchedAgentToken(entry) {
  const comm = (entry.comm || '').toLowerCase();
  const argv0 = ((entry.command || '').trim().split(/\s+/)[0] || '');
  const argv0base = basename(argv0).toLowerCase();
  for (const t of AGENT_TOKENS) {
    if (comm.includes(t)) return t;
    if (argv0base === t) return t;
  }
  return null;
}

// 在进程链里猜测「Agent 会话进程」：从自身向上，第一个命中 agent token
// 且不是 harness 自身的节点。返回 { agent, candidates }。
function guessAgentPid(chain) {
  const candidates = [];
  for (const e of chain) {
    if (isSelf(e)) continue;
    const token = matchedAgentToken(e);
    if (token) candidates.push({ ...e, token });
  }
  return { agent: candidates[0] || null, candidates };
}

// 与 Agent 相关的 env：精确列出 Claude 的关键变量，外加任何 CURSOR*/CODEX* 前缀。
const CLAUDE_ENV_KEYS = [
  'CLAUDECODE',
  'CLAUDE_CODE_SESSION_ID',
  'CLAUDE_CODE_CHILD_SESSION',
  'CLAUDE_PROJECT_DIR',
  'CLAUDE_CODE_ENTRYPOINT',
  'CLAUDE_CODE_REMOTE',
  'CLAUDE_CONFIG_DIR',
  'CLAUDE_PLUGIN_ROOT',
  'CLAUDE_ENV_FILE',
];
const OTHER_ENV_PREFIXES = ['CURSOR', 'CODEX'];

function agentEnv() {
  const out = {};
  for (const k of CLAUDE_ENV_KEYS) {
    if (process.env[k] !== undefined) out[k] = process.env[k];
  }
  for (const k of Object.keys(process.env)) {
    if (OTHER_ENV_PREFIXES.some((p) => k.startsWith(p))) out[k] = process.env[k];
  }
  return out;
}

// 全量 env（敏感值打码，仅保留键名与长度，方便看「有哪些键」而不泄露密钥）。
function redactedEnv() {
  const SENSITIVE = /(KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|AUTH)/i;
  const out = {};
  for (const [k, v] of Object.entries(process.env)) {
    out[k] = SENSITIVE.test(k) ? `<redacted len=${String(v).length}>` : v;
  }
  return out;
}

function appendJsonl(file, obj) {
  ensureLogs();
  fs.appendFileSync(path.join(LOGS_DIR, file), JSON.stringify(obj) + '\n');
}

function writeJson(file, obj) {
  ensureLogs();
  fs.writeFileSync(
    path.join(LOGS_DIR, file),
    JSON.stringify(obj, null, 2) + '\n'
  );
}

function writePidFile(info) {
  ensureLogs();
  fs.writeFileSync(PID_FILE, JSON.stringify(info, null, 2) + '\n');
}

function readPidFile() {
  try {
    return JSON.parse(fs.readFileSync(PID_FILE, 'utf8'));
  } catch {
    return null;
  }
}

// kill -0 探活：返回 'alive' | 'dead' | 'unknown'
function probeAlive(pid) {
  if (!pid) return 'unknown';
  try {
    process.kill(Number(pid), 0);
    return 'alive';
  } catch (e) {
    if (e.code === 'EPERM') return 'alive'; // 存在但无权限发信号
    if (e.code === 'ESRCH') return 'dead';
    return 'unknown';
  }
}

module.exports = {
  DEMO_ROOT,
  LOGS_DIR,
  PID_FILE,
  nowIso,
  ensureLogs,
  processChain,
  basename,
  guessAgentPid,
  agentEnv,
  redactedEnv,
  appendJsonl,
  writeJson,
  writePidFile,
  readPidFile,
  probeAlive,
};
