// 快捷键工具：在「弹窗内」识别/录入快捷键组合。
// 规范字符串格式：修饰键(按 cmd,ctrl,alt,shift 顺序) + 主键，用 "+" 连接，全小写。
//   例如 "cmd+d"、"cmd+shift+d"。空串表示「无/关闭」。
// 主键统一用物理键(e.code)归一，避免 Shift 改变 e.key 导致的不一致。

export interface ShortcutSpec {
  cmd: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: string; // 归一后的主键，如 "d" / "1" / "[" / "enter"
}

// e.code → 归一主键。仅覆盖常见可作快捷键的物理键。
function codeToKey(code: string): string | null {
  if (code.startsWith("Key")) return code.slice(3).toLowerCase(); // KeyD → d
  if (code.startsWith("Digit")) return code.slice(5); // Digit1 → 1
  const map: Record<string, string> = {
    BracketLeft: "[",
    BracketRight: "]",
    Enter: "enter",
    NumpadEnter: "enter",
    Space: "space",
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backslash: "\\",
    Semicolon: ";",
    Quote: "'",
    Minus: "-",
    Equal: "=",
    Backquote: "`",
  };
  return map[code] ?? null;
}

const MODIFIER_KEYS = new Set(["Meta", "Control", "Alt", "Shift"]);

// 是否为「纯修饰键」按下（录制时应忽略，继续等待主键）。
export function isModifierOnly(e: KeyboardEvent): boolean {
  return MODIFIER_KEYS.has(e.key);
}

// 从键盘事件解析组合；纯修饰键或无法归一的主键返回 null。
export function eventToSpec(e: KeyboardEvent): ShortcutSpec | null {
  if (isModifierOnly(e)) return null;
  const key = codeToKey(e.code);
  if (!key) return null;
  return {
    cmd: e.metaKey,
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    key,
  };
}

export function specToString(s: ShortcutSpec): string {
  const parts: string[] = [];
  if (s.cmd) parts.push("cmd");
  if (s.ctrl) parts.push("ctrl");
  if (s.alt) parts.push("alt");
  if (s.shift) parts.push("shift");
  parts.push(s.key);
  return parts.join("+");
}

export function parseShortcut(spec: string): ShortcutSpec | null {
  if (!spec) return null;
  const tokens = spec.toLowerCase().split("+");
  const key = tokens.pop();
  if (!key) return null;
  return {
    cmd: tokens.includes("cmd"),
    ctrl: tokens.includes("ctrl"),
    alt: tokens.includes("alt"),
    shift: tokens.includes("shift"),
    key,
  };
}

// 主键的人类可读符号。
function keySymbol(key: string): string {
  const map: Record<string, string> = {
    enter: "↩",
    space: "␣",
  };
  if (map[key]) return map[key];
  return key.length === 1 ? key.toUpperCase() : key;
}

// 规范字符串 → 展示文案（如 "⌘⇧D"）；空串 → "无"。
export function formatShortcut(spec: string): string {
  const s = parseShortcut(spec);
  if (!s) return "无";
  let out = "";
  if (s.ctrl) out += "⌃";
  if (s.alt) out += "⌥";
  if (s.shift) out += "⇧";
  if (s.cmd) out += "⌘";
  out += keySymbol(s.key);
  return out;
}

// 事件是否命中某规范快捷键（精确匹配四个修饰键 + 主键）。
export function matchShortcut(e: KeyboardEvent, spec: string): boolean {
  const want = parseShortcut(spec);
  if (!want) return false;
  const got = eventToSpec(e);
  if (!got) return false;
  return (
    got.cmd === want.cmd &&
    got.ctrl === want.ctrl &&
    got.alt === want.alt &&
    got.shift === want.shift &&
    got.key === want.key
  );
}

// 校验录入的组合是否可用；返回错误文案，null 表示通过。
// 规则：必须含 ⌘ 或 ⌃；不得与弹窗内既有快捷键 / 常用系统编辑键冲突。
export function shortcutConflict(s: ShortcutSpec): string | null {
  const mod = s.cmd || s.ctrl;
  if (!mod) return "请至少包含 ⌘ 或 ⌃";

  if (s.key === "enter") return "与「提交 / 下一题」(⌘↩) 冲突";
  if (s.key === "w") return "与「取消」(⌘W) 冲突";
  if (s.key === "[" || s.key === "]") return "与「上一题 / 下一题」(⌘[ ⌘]) 冲突";
  if (s.key >= "1" && s.key <= "9") return "与「选项快捷键」(⌘1–9) 冲突";

  // 常用系统/文本编辑键：仅在「⌘/⌃ + 字母」且无 ⌥⇧ 时判冲突，避免误伤 ⌘⇧V 等。
  if (mod && !s.alt && !s.shift && ["a", "c", "v", "x", "z"].includes(s.key)) {
    return `与系统编辑快捷键 (⌘${s.key.toUpperCase()}) 冲突，建议加 ⇧ 或换一个`;
  }
  return null;
}
