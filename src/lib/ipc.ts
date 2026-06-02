import { invoke } from "@tauri-apps/api/core";
import type { PopupInit, PopupSubmission } from "./types";

export const popupInit = () => invoke<PopupInit>("popup_init");

export const popupReady = () => invoke<void>("popup_ready");

export const submitPopup = (submission: PopupSubmission) =>
  invoke<void>("submit_popup", { submission });

export const cancelPopup = () => invoke<void>("cancel_popup");
