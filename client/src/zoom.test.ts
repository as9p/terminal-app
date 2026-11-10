import { describe, expect, it, vi } from "vitest";

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
    const views = [1, 2, 3].map(() => ({ setFontSize: vi.fn() }));
    applyZoom({ all: () => views as unknown as TerminalView[] }, 15);
    for (const view of views) expect(view.setFontSize).toHaveBeenCalledWith(15);
  });

  it("spacing_keeps_whole_cells", () => {
    expect(spacingFor(14, MEASURED_AT_14)).toBe(1);
    // 18 device pixels at 15 px is already whole, so no spacing (the stage 1 note's 18 plus 1).
    expect(spacingFor(15, MEASURED_AT_14)).toBe(0);
    for (let size = 8; size <= 40; size++) {
      const natural = Math.round(((MEASURED_AT_14 * size) / 14) * 1000) / 1000;
      expect(Math.floor(natural) + spacingFor(size, MEASURED_AT_14)).toBe(Math.ceil(natural));
    }
  });

  it("zoom_survives_restart", async () => {
    localStorage.clear();
    await saveSettings({ fontSize: 17 });
    expect((await loadSettings()).fontSize).toBe(17);
    expect((await loadSettings()).dim).toBe(false);
  });
});
