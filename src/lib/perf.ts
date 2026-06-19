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

/**
 * Enable reporting. By default flush everything buffered before perf state was known (cold path:
 * the buffered marks belong to this request). Pass `dropBuffered` for the **warm adoption** path:
 * the buffered marks (fe.bootstrap/fe.mounted/standby popup_init) happened during *prewarm*, not
 * for this request — flushing them under the request's perf_id would pollute the timeline
 * (negative "page boot"), so we discard them and only report marks made after adoption.
 */
export function enable(dropBuffered = false): void {
  if (enabled) return;
  enabled = true;
  const pending = buffered.splice(0);
  if (dropBuffered) return;
  for (const m of pending) {
    void perfMark(m.stage, m.ts).catch(() => {});
  }
}
