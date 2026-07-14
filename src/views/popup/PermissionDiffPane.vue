<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { PermissionDiffModel } from "../../lib/types";
import {
  diffLinePrefix,
  hunkLabel,
  permissionDisplayPath,
  snapshotStatusKey,
  snapshotTime,
} from "./permissionDiff";

const { t } = useI18n();
const props = defineProps<{
  model: PermissionDiffModel;
  loading: boolean;
  workspace: string;
}>();

function displayPath(path: string): string {
  return permissionDisplayPath(path, props.workspace);
}
</script>

<template>
  <section class="permission-diff" aria-live="polite">
    <header class="permission-diff-summary">
      <strong>{{ t("popup.permissionDiff.title") }}</strong>
      <span v-if="model.totalFiles" class="permission-diff-count">
        {{ t("popup.permissionDiff.files", { n: model.totalFiles }) }}
      </span>
      <span v-if="model.additions" class="permission-diff-stat additions">
        {{ t("popup.permissionDiff.additions", { n: model.additions }) }}
      </span>
      <span v-if="model.deletions" class="permission-diff-stat deletions">
        {{ t("popup.permissionDiff.deletions", { n: model.deletions }) }}
      </span>
    </header>

    <div
      class="permission-diff-status"
      :class="[`status-${model.snapshotStatus}`, { loading }]"
    >
      <span class="permission-diff-status-dot" aria-hidden="true"></span>
      <span>
        {{
          loading
            ? t("popup.permissionDiff.loadingSnapshot")
            : t(snapshotStatusKey(model.snapshotStatus))
        }}
      </span>
      <span v-if="snapshotTime(model.snapshotAtMs)" class="permission-diff-time">
        {{
          t("popup.permissionDiff.snapshotAt", {
            time: snapshotTime(model.snapshotAtMs),
          })
        }}
      </span>
    </div>

    <div v-for="file in model.files" :key="`${file.oldPath ?? ''}:${file.newPath}`" class="permission-diff-file">
      <header class="permission-diff-file-header">
        <span
          v-if="file.oldPath && file.oldPath !== file.newPath"
          class="permission-diff-path"
          :title="`${file.oldPath} → ${file.newPath}`"
        >
          {{ displayPath(file.oldPath) }}
          <span class="permission-diff-move" aria-hidden="true">→</span>
          {{ displayPath(file.newPath) }}
        </span>
        <span
          v-else
          class="permission-diff-path"
          :title="displayPath(file.newPath) === file.newPath ? undefined : file.newPath"
        >
          {{ displayPath(file.newPath) }}
        </span>
        <span
          v-if="file.snapshotStatus !== model.snapshotStatus"
          class="permission-diff-file-status"
        >
          {{ t(snapshotStatusKey(file.snapshotStatus)) }}
        </span>
      </header>

      <div v-for="(hunk, hunkIndex) in file.hunks" :key="hunkIndex" class="permission-diff-hunk">
        <div class="permission-diff-hunk-header">{{ hunkLabel(hunk) }}</div>
        <div
          v-for="(line, lineIndex) in hunk.lines"
          :key="lineIndex"
          class="permission-diff-line"
          :class="`kind-${line.kind}`"
        >
          <span class="permission-diff-line-no">{{ line.oldLine ?? "" }}</span>
          <span class="permission-diff-line-no">{{ line.newLine ?? "" }}</span>
          <span class="permission-diff-prefix">{{ diffLinePrefix(line.kind) }}</span>
          <code>{{ line.text }}</code>
        </div>
      </div>
    </div>

    <div v-if="model.truncated" class="permission-diff-omitted">
      {{
        t("popup.permissionDiff.omitted", {
          files: model.omittedFiles,
          hunks: model.omittedHunks,
          lines: model.omittedLines,
        })
      }}
    </div>
  </section>
</template>
