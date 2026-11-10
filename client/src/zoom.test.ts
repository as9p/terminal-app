import { describe, expect, it, vi } from "vitest";

import type { Split, Tab } from "./connection";
import { Panes } from "./panes";
import { loadSettings, saveSettings } from "./settings";
import type { TerminalView } from "./terminal";
import { applyZoom, nextSize, spacingFor } from "./zoom";

// The natural cell width at 14 px from the stage 1 handoff: 17 device pixels drawn at ratio 2,
// from 16.8 rounded up.
const MEASURED_AT_14 = 16.8;

describe("zoom", () => {
  it("zoom_steps_one_px", () => {
    expect(nextSize(14, 1)).toBe(15);
    expect(nextSize(14, -1)).toBe(13);
  });

  it("zoom_clamps_to_8_and_40", () => {
    expect(nextSize(40, 1)).toBe(40);
    expect(nextSize(8, -1)).toBe(8);
    expect(nextSize(300, -1)).toBe(39);
    expect(nextSize(Number.NaN, 1)).toBe(15);
  });

  it("reset_goes_to_14", () => {
    expect(nextSize(30, 0)).toBe(14);
    expect(nextSize(8, 0)).toBe(14);
  });

  it("zoom_applies_to_every_terminal", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const views = new Map<string, { setFontSize: ReturnType<typeof vi.fn> }>();
    const panes = new Panes(root, (_container, session) => {
      const view = { setFontSize: vi.fn(), focus: vi.fn(), dispose: vi.fn(), setDimmed: vi.fn() };
      views.set(session, view);
      return view as unknown as TerminalView;
    });
    const split: Split = { type: "row", children: [{ type: "leaf", session: "a" }, { type: "leaf", session: "b" }], sizes: [1, 1] };
    const first: Tab = { id: "t1", name: null, pinned: false, split, last_used: 0 };
    const second: Tab = { id: "t2", name: null, pinned: false, split: { type: "leaf", session: "c" }, last_used: 0 };
    panes.show(first);
    // Tab t1's two views are parked, hidden, while t2 is on screen.
    panes.show(second);
    expect(root.querySelector(".parked")?.children.length).toBe(2);
    applyZoom(panes, 15);
    expect([...views.keys()].sort()).toEqual(["a", "b", "c"]);
    for (const view of views.values()) expect(view.setFontSize).toHaveBeenCalledWith(15);
    root.remove();
  });

  it("spacing_keeps_whole_cells", () => {
    expect(spacingFor(14, MEASURED_AT_14)).toBe(1);
    // 18 device pixels at 15 px is already whole, so no spacing (the stage 1 note's 18 plus 1).
    expect(spacingFor(15, MEASURED_AT_14)).toBe(0);
    // At 16.8 device pixels per cell at 14 px, a cell is 1.2 device pixels per px of size.
    expect(spacingFor(8, MEASURED_AT_14)).toBe(1); // 9.6 drawn as 10
    expect(spacingFor(10, MEASURED_AT_14)).toBe(0); // 12
    expect(spacingFor(13, MEASURED_AT_14)).toBe(1); // 15.6 drawn as 16
    expect(spacingFor(20, MEASURED_AT_14)).toBe(0); // 24
    expect(spacingFor(21, MEASURED_AT_14)).toBe(1); // 25.2 drawn as 26
    expect(spacingFor(40, MEASURED_AT_14)).toBe(0); // 48
    // A display measuring 16 at 14 px: 16 and 24 are whole, 17.14 is not.
    expect(spacingFor(14, 16)).toBe(0);
    expect(spacingFor(21, 16)).toBe(0);
    expect(spacingFor(15, 16)).toBe(1);
  });

  it("zoom_survives_restart", async () => {
    localStorage.clear();
    await saveSettings({ fontSize: 17 });
    expect((await loadSettings()).fontSize).toBe(17);
    expect((await loadSettings()).dim).toBe(false);
  });
});
