// Text zoom: every terminal at once, 1 px a step, 8 to 40 px, back to 14. The sidebar keeps its
// own sizes. No popup or readout shows the size.

import type { Panes } from "./panes";

export const ZOOM_DEFAULT = 14;
export const ZOOM_MIN = 8;
export const ZOOM_MAX = 40;

/** Whole pixels between the limits. Anything that is not a number gives the default. */
export function clampSize(px: number): number {
  if (!Number.isFinite(px)) return ZOOM_DEFAULT;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(px)));
}

/** One step up or down from `current`, or back to 14 for step 0. */
export function nextSize(current: number, step: 1 | -1 | 0): number {
  if (step === 0) return ZOOM_DEFAULT;
  return clampSize(clampSize(current) + step);
}

export function applyZoom(panes: Pick<Panes, "all">, size: number): void {
  for (const view of panes.all()) view.setFontSize(size);
}

/**
 * xterm's `letterSpacing` for `size`, in device pixels. xterm rounds a glyph's width down and
 * kitty rounds it up, so the spacing is what lifts the rounded-down width to the rounded-up one,
 * and a width that is already whole gets none. `measuredCellWidth` is the natural cell width in
 * device pixels at 14 px (16.8 at ratio 2 in the stage 1 handoff); it scales with the size.
 */
export function spacingFor(size: number, measuredCellWidth: number): number {
  // Rounded to a thousandth first, so 16.8 * 15 / 14 counts as 18 and not as just over it.
  const natural = Math.round(((measuredCellWidth * size) / ZOOM_DEFAULT) * 1000) / 1000;
  return Math.ceil(natural) - Math.floor(natural);
}
