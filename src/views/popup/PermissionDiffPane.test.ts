import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";
import { i18n } from "../../i18n";
import type { PermissionDiffModel } from "../../lib/types";
import PermissionDiffPane from "./PermissionDiffPane.vue";
import {
  diffLinePrefix,
  hunkLabel,
  permissionDisplayPath,
  snapshotStatusKey,
} from "./permissionDiff";

function model(): PermissionDiffModel {
  return {
    requestId: "r1",
    snapshotStatus: "snapshot_ready",
    snapshotAtMs: 1_700_000_000_000,
    totalFiles: 1,
    additions: 1,
    deletions: 1,
    omittedFiles: 0,
    omittedHunks: 0,
    omittedLines: 0,
    truncated: false,
    files: [
      {
        changeKind: "moved",
        oldPath: "/repo/old.ts",
        newPath: "/repo/new.ts",
        snapshotStatus: "snapshot_ready",
        additions: 1,
        deletions: 1,
        omittedHunks: 0,
        omittedLines: 0,
        hunks: [
          {
            oldStart: 3,
            newStart: 3,
            header: "function demo",
            lines: [
              { kind: "delete", oldLine: 3, text: "<script>bad()</script>" },
              { kind: "add", newLine: 3, text: "safe()" },
            ],
          },
        ],
      },
    ],
  };
}

describe("PermissionDiffPane", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });

  it("renders structured moved-file diff and keeps source text inert", () => {
    const wrapper = mount(PermissionDiffPane, {
      props: { model: model(), loading: false, workspace: "/repo" },
      global: { plugins: [i18n] },
    });

    expect(wrapper.find(".permission-diff-path").text()).toContain("old.ts → new.ts");
    expect(wrapper.find(".permission-diff-hunk-header").text()).toBe(
      "@@ -3 +3 @@ function demo"
    );
    expect(wrapper.findAll(".permission-diff-line")).toHaveLength(2);
    expect(wrapper.find("script").exists()).toBe(false);
    expect(wrapper.find("code").text()).toBe("<script>bad()</script>");
    expect(wrapper.text()).toContain("Combined with a local file snapshot");
  });

  it("shows exact truncation counters", () => {
    const value = model();
    value.truncated = true;
    value.omittedFiles = 2;
    value.omittedHunks = 3;
    value.omittedLines = 44;
    const wrapper = mount(PermissionDiffPane, {
      props: { model: value, loading: true, workspace: "/repo" },
      global: { plugins: [i18n] },
    });
    expect(wrapper.find(".permission-diff-status").classes()).toContain("loading");
    expect(wrapper.find(".permission-diff-omitted").text()).toContain(
      "Omitted 2 files, 3 hunks, and 44 lines"
    );
  });
});

describe("permission diff formatting", () => {
  it("formats stable line, hunk, and status labels", () => {
    expect(diffLinePrefix("add")).toBe("+");
    expect(diffLinePrefix("delete")).toBe("−");
    expect(
      hunkLabel({ oldStart: null, newStart: 7, header: "", lines: [] })
    ).toBe("@@ -? +7 @@");
    expect(snapshotStatusKey("protected_path")).toBe(
      "popup.permissionDiff.status.protected_path"
    );
  });

  it("shows cwd files relatively while keeping outside paths absolute", () => {
    expect(permissionDisplayPath("/repo/src/a.ts", "/repo")).toBe("src/a.ts");
    expect(permissionDisplayPath("/repo/src/../a.ts", "/repo/")).toBe("a.ts");
    expect(permissionDisplayPath("/repo-other/a.ts", "/repo")).toBe(
      "/repo-other/a.ts"
    );
    expect(permissionDisplayPath("/outside/a.ts", "/repo")).toBe(
      "/outside/a.ts"
    );
    expect(permissionDisplayPath("src/a.ts", "/repo")).toBe("src/a.ts");
  });
});
