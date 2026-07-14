<script setup lang="ts">
// Agent 权限确认面板（confirm 交互）：标题 + 理由 + 工具详情 + 单选动作 + 可选备注输入。
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";
import PermissionDiffPane from "./PermissionDiffPane.vue";

const { t } = useI18n();
const {
  confirmRequest,
  confirmChoiceIndex,
  confirmComment,
  confirmInput,
  showConfirmInput,
  confirmDetailHtml,
  confirmToolName,
  permissionEdit,
  permissionDiff,
  permissionDiffLoading,
  selectConfirmChoice,
  onContentClick,
} = usePopupContext();
</script>

<template>
  <section v-if="confirmRequest" class="confirm-request">
    <h1 class="confirm-request-title">{{ confirmRequest.title }}</h1>
    <p v-if="confirmRequest.detail.summary" class="confirm-reason">
      <strong>{{ t("popup.permissionReason") }}</strong>
      {{ confirmRequest.detail.summary }}
    </p>
    <section class="confirm-tool">
      <header class="confirm-tool-header">{{ confirmToolName }}</header>
      <PermissionDiffPane
        v-if="permissionEdit && permissionDiff"
        :model="permissionDiff"
        :loading="permissionDiffLoading"
        :workspace="permissionEdit.workspace"
      />
      <details
        v-if="permissionEdit && confirmRequest.detail.bodyMd"
        class="confirm-raw-details"
      >
        <summary>{{ t("popup.permissionDiff.originalParams") }}</summary>
        <div
          class="markdown-body confirm-detail"
          v-html="confirmDetailHtml"
          @click="onContentClick"
        ></div>
      </details>
      <div
        v-else-if="confirmRequest.detail.bodyMd"
        class="markdown-body confirm-detail"
        v-html="confirmDetailHtml"
        @click="onContentClick"
      ></div>
    </section>
    <div class="confirm-options" role="radiogroup" :aria-label="confirmRequest.title">
      <div
        v-for="(choice, index) in confirmRequest.choices"
        :key="choice.id"
        class="option single confirm-option"
        :class="[
          `role-${choice.role}`,
          { selected: confirmChoiceIndex === index },
        ]"
        role="radio"
        tabindex="0"
        :aria-checked="confirmChoiceIndex === index"
        @click="selectConfirmChoice(index)"
        @keydown.enter.prevent="selectConfirmChoice(index)"
        @keydown.space.prevent="selectConfirmChoice(index)"
      >
        <span class="check radio" aria-hidden="true"></span>
        <span class="label confirm-option-label">
          <span>{{ choice.label }}</span>
          <small v-if="choice.description">{{ choice.description }}</small>
        </span>
        <kbd v-if="index < 9" class="opt-sc">⌘{{ index + 1 }}</kbd>
      </div>
    </div>
    <label v-if="showConfirmInput && confirmInput" class="confirm-input-block">
      <span>{{ confirmInput.label }}</span>
      <textarea
        v-model="confirmComment"
        class="textarea"
        :maxlength="confirmInput.maxChars"
        :placeholder="confirmInput.placeholder"
        rows="4"
      ></textarea>
      <small>{{ confirmComment.length }} / {{ confirmInput.maxChars }}</small>
    </label>
  </section>
</template>
