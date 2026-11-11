// Typing always lands in the terminal. After anything the pointer does, and after a tree change,
// focus goes back to the current pane, unless a field such as a rename box is holding it.

import type { Panes } from "./panes";

const held = new WeakSet<HTMLElement>();
let installed: { root: HTMLElement; panes: Pick<Panes, "focusCurrent">; timer: number } | undefined;

/** Puts focus back on the current pane unless a held field has it. */
export function refocus(): void {
  if (!installed) return;
  const active = document.activeElement;
  if (active instanceof HTMLElement && held.has(active)) return;
  installed.panes.focusCurrent();
}

// After the click that follows a mouseup, so a row click selects its tab before focus moves,
// and a double click that opens a field has called holdFocus by then.
function refocusSoon(): void {
  if (!installed || installed.timer) return;
  installed.timer = window.setTimeout(() => {
    if (!installed) return;
    installed.timer = 0;
    refocus();
  }, 0);
}

export function installFocusRule(root: HTMLElement, panes: Pick<Panes, "focusCurrent">): void {
  uninstallFocusRule();
  installed = { root, panes, timer: 0 };
  root.addEventListener("mouseup", refocusSoon);
  root.addEventListener("drop", refocusSoon);
  root.addEventListener("contextmenu", refocusSoon);
  // A context menu closing gives focus back to the page, and so does coming back to the window.
  window.addEventListener("focus", refocusSoon);
}

/** For tests, and for a window that is torn down. */
export function uninstallFocusRule(): void {
  if (!installed) return;
  clearTimeout(installed.timer);
  installed.root.removeEventListener("mouseup", refocusSoon);
  installed.root.removeEventListener("drop", refocusSoon);
  installed.root.removeEventListener("contextmenu", refocusSoon);
  window.removeEventListener("focus", refocusSoon);
  installed = undefined;
}

/** Focuses `field` and keeps focus there until Enter or Escape, then gives it back to the pane. */
export function holdFocus(field: HTMLElement): void {
  held.add(field);
  const onKey = (event: KeyboardEvent) => {
    if (event.isComposing || (event.key !== "Enter" && event.key !== "Escape")) return;
    held.delete(field);
    field.removeEventListener("keydown", onKey);
    refocus();
  };
  field.addEventListener("keydown", onKey);
  field.focus();
}
