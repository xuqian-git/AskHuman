import { mount } from "@vue/test-utils";
import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import SequentialPane from "./SequentialPane.vue";
import { PopupCtxKey, type PopupContext } from "./context";

function mountPane(whatsNext: boolean) {
  const options = [
    {
      text: "Run todo: ship the fix",
      recommended: false,
      todoId: "todo-1",
    },
    { text: "Review logs", recommended: true },
  ];
  const ctx = {
    request: ref({ whatsNext, isMarkdown: false }),
    showQuestionHeader: ref(false),
    showDescription: ref(false),
    questionHeaderLabel: ref("Question"),
    qHeaderRef: ref<HTMLElement | null>(null),
    transitionName: ref("none"),
    onQuestionEntered: vi.fn(),
    current: ref(0),
    currentQuestion: ref({ message: "", predefinedOptions: options }),
    renderedHtml: ref(""),
    viewSource: ref(false),
    onContentClick: vi.fn(),
    chosen: ref<string[]>([]),
    single: ref(false),
    selectOnly: ref(true),
    optionHotkey: vi.fn(() => null),
    toggle: vi.fn(),
  } as unknown as PopupContext;

  return mount(SequentialPane, {
    global: {
      plugins: [i18n],
      provide: { [PopupCtxKey as symbol]: ctx },
      stubs: { AnswerComposer: true, Transition: false },
    },
  });
}

describe("SequentialPane todo badge", () => {
  it("marks only todo options in a whats-next request", () => {
    const wrapper = mountPane(true);
    const rows = wrapper.findAll(".option");
    expect(rows[0].get(".todo-option-badge").text()).toBe("TODO");
    expect(rows[0].text()).toContain("ship the fix");
    expect(rows[0].text()).not.toContain("Run todo:");
    expect(rows[1].find(".todo-option-badge").exists()).toBe(false);
  });

  it("does not mark todo-id options outside whats-next", () => {
    const wrapper = mountPane(false);
    expect(wrapper.find(".todo-option-badge").exists()).toBe(false);
    expect(wrapper.text()).toContain("Run todo: ship the fix");
  });
});
