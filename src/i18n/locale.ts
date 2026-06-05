import type { UiLanguage } from "../lib/types";

export type Locale = "en" | "zh";

// 把配置语言解析为实际 locale：显式 en/zh 直用；auto → 看系统/浏览器语言。
// 规则（与后端一致）：首选语言以 "zh" 开头 → 中文，否则英文。
export function resolveLocale(lang: UiLanguage | string | undefined): Locale {
  if (lang === "en") return "en";
  if (lang === "zh") return "zh";
  // auto / 未知：跟随系统
  const sys =
    (typeof navigator !== "undefined" &&
      (navigator.language || navigator.languages?.[0])) ||
    "en";
  return sys.toLowerCase().startsWith("zh") ? "zh" : "en";
}
