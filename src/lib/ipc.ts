import { invoke } from "@tauri-apps/api/core";
import type {
  AppConfig,
  HookStatus,
  PopupInit,
  PopupSubmission,
  TelegramTestArgs,
  ThemeMode,
} from "./types";

export const popupInit = () => invoke<PopupInit>("popup_init");

export const submitPopup = (submission: PopupSubmission) =>
  invoke<void>("submit_popup", { submission });

export const cancelPopup = () => invoke<void>("cancel_popup");

export const getSettings = () => invoke<AppConfig>("get_settings");

export const saveSettings = (config: AppConfig) =>
  invoke<void>("save_settings", { config });

export const getPrompt = () => invoke<string>("get_prompt");

export const setTheme = (theme: ThemeMode) =>
  invoke<void>("set_theme", { theme });

export const cursorHookStatus = () => invoke<HookStatus>("cursor_hook_status");

export const cursorHookInstall = () => invoke<string>("cursor_hook_install");

export const cursorHookUninstall = () => invoke<string>("cursor_hook_uninstall");

export const cursorHookReveal = () => invoke<void>("cursor_hook_reveal");

export const telegramTest = (args: TelegramTestArgs) =>
  invoke<string>("telegram_test", { args });
