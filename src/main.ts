import { createApp } from "vue";
import App from "./App.vue";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/controls.css";
import { i18n, applyLanguage } from "./i18n";
import { getSettings } from "./lib/ipc";
import { mark as perfMarkFe } from "./lib/perf";

async function bootstrap() {
  perfMarkFe("fe.bootstrap");
  // 先按系统/浏览器语言兜底，再读配置校正（auto/en/zh）。
  applyLanguage("auto");
  try {
    const s = await getSettings();
    applyLanguage(s.config.general.language);
  } catch {
    /* 读取失败：保持兜底语言 */
  }
  createApp(App).use(i18n).mount("#app");
  perfMarkFe("fe.mounted");
}

bootstrap();
