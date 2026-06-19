// Frontend perf instrumentation for the popup launch path (spec popup-launch-performance §7).
//
// Marks are buffered until `enable()` is called (we only learn perf is on after `popup_init`
// resolves). Once enabled, the buffer is flushed and later marks report immediately. When perf is
// off `enable()` is never called, so marks just accumulate harmlessly in memory and no IPC fires.

import { perfMark } from "./ipc";

type Mark = { stage: string; ts: number };

const buffered: Mark[] = [];
let enabled = false;

/** Record a milestone at the current time (uses page clock, not the IPC round trip). */
export function mark(stage: string): void {
  const ts = Date.now();
  if (enabled) {
    void perfMark(stage, ts).catch(() => {});
  } else {
    buffered.push({ stage, ts });
  }
}

/** Enable reporting and flush everything buffered before perf state was known. */
export function enable(): void {
  if (enabled) return;
  enabled = true;
  for (const m of buffered.splice(0)) {
    void perfMark(m.stage, m.ts).catch(() => {});
  }
}
