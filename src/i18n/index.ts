import { createI18n } from "vue-i18n";
import en from "./en";
import zh from "./zh";
import { resolveLocale, type Locale } from "./locale";
import type { UiLanguage } from "../lib/types";

// 全局 i18n 实例。Composition API 模式（legacy:false），回退英文。
export const i18n = createI18n({
  legacy: false,
  locale: "en",
  fallbackLocale: "en",
  messages: { en, zh },
});

// 按「配置语言」设置实际 locale（auto→系统）。供启动与设置变更时调用。
export function applyLanguage(lang: UiLanguage | string | undefined): Locale {
  const loc = resolveLocale(lang);
  i18n.global.locale.value = loc;
  return loc;
}

export { resolveLocale };
export type { Locale };
