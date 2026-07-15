import { describe, expect, it } from "vitest";
import {
  canComposerDock,
  composerHomeVisibleRatio,
  isComposerHomeFullyVisible,
  resolveComposerDocked,
  type ComposerDockGeometry,
} from "./composerDock";

function geometry(overrides: Partial<ComposerDockGeometry> = {}): ComposerDockGeometry {
  return {
    homeTop: 300,
    homeBottom: 380,
    viewportTop: 100,
    viewportBottom: 500,
    viewportBottomAfterUndock: 500,
    ...overrides,
  };
}

describe("canComposerDock", () => {
  it("requires an upward scroll even after manual activation", () => {
    expect(canComposerDock(true, true, false, false)).toBe(false);
    expect(canComposerDock(true, true, false, true)).toBe(true);
  });

  it("requires an upward scroll after an automatically focused owner was fully seen", () => {
    expect(canComposerDock(true, false, false, false)).toBe(false);
    expect(canComposerDock(true, false, true, false)).toBe(false);
    expect(canComposerDock(true, false, true, true)).toBe(true);
  });

  it("does not start docking after the inline owner has blurred", () => {
    expect(canComposerDock(false, true, true, true)).toBe(false);
  });
});

describe("resolveComposerDocked", () => {
  it("does not dock an owner that has never been visible inline", () => {
    expect(
      resolveComposerDocked(false, false, geometry({ homeTop: 520, homeBottom: 600 }))
    ).toBe(false);
  });

  it("docks when the input home crosses the viewport bottom", () => {
    expect(
      resolveComposerDocked(false, true, geometry({ homeTop: 450, homeBottom: 510 }))
    ).toBe(true);
  });

  it("does not dock when the input leaves through the viewport top", () => {
    expect(
      resolveComposerDocked(false, true, geometry({ homeTop: 60, homeBottom: 140 }))
    ).toBe(false);
    expect(
      resolveComposerDocked(true, true, geometry({ homeTop: 60, homeBottom: 140 }))
    ).toBe(false);
  });

  it("uses a return gap so the boundary cannot oscillate", () => {
    expect(
      resolveComposerDocked(true, true, geometry({ homeTop: 410, homeBottom: 495 }))
    ).toBe(true);
    expect(
      resolveComposerDocked(true, true, geometry({ homeTop: 400, homeBottom: 490 }))
    ).toBe(false);
  });

  it("bases return on the input home rather than a tall attachment area", () => {
    const inputHome = geometry({ homeTop: 350, homeBottom: 470 });
    expect(resolveComposerDocked(true, true, inputHome)).toBe(false);
  });

  it("returns as soon as the viewport released by the dock can contain the home", () => {
    expect(
      resolveComposerDocked(
        true,
        true,
        geometry({
          homeTop: 490,
          homeBottom: 570,
          viewportBottom: 500,
          viewportBottomAfterUndock: 650,
        })
      )
    ).toBe(false);
  });

  it("keeps a taller-than-viewport input docked until its home can fit", () => {
    expect(
      resolveComposerDocked(
        true,
        true,
        geometry({ homeTop: 110, homeBottom: 560 })
      )
    ).toBe(true);
  });
});

describe("composerHomeVisibleRatio", () => {
  it("measures partial visibility within the content viewport", () => {
    expect(composerHomeVisibleRatio(geometry())).toBe(1);
    expect(
      composerHomeVisibleRatio(geometry({ homeTop: 460, homeBottom: 540 }))
    ).toBe(0.5);
    expect(
      composerHomeVisibleRatio(geometry({ homeTop: 500, homeBottom: 580 }))
    ).toBe(0);
  });
});

describe("isComposerHomeFullyVisible", () => {
  it("rejects a home clipped at either edge", () => {
    expect(isComposerHomeFullyVisible(geometry())).toBe(true);
    expect(
      isComposerHomeFullyVisible(geometry({ homeTop: 80, homeBottom: 180 }))
    ).toBe(false);
    expect(
      isComposerHomeFullyVisible(geometry({ homeTop: 460, homeBottom: 540 }))
    ).toBe(false);
  });
});
