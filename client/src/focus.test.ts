import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { holdFocus, installFocusRule, refocus, uninstallFocusRule } from "./focus";

let root: HTMLElement;
let row: HTMLElement;
let terminal: HTMLTextAreaElement;
let focusCurrent: ReturnType<typeof vi.fn<() => void>>;

beforeEach(() => {
  vi.useFakeTimers();
  root = document.createElement("div");
  row = document.createElement("div");
  row.className = "row";
  terminal = document.createElement("textarea");
  root.append(row, terminal);
  document.body.append(root);
  focusCurrent = vi.fn<() => void>(() => terminal.focus());
  installFocusRule(root, { focusCurrent });
});

afterEach(() => {
  uninstallFocusRule();
  root.remove();
  vi.useRealTimers();
});

function press(target: HTMLElement, key: string): void {
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
}

describe("focus rule", () => {
  it("click_on_sidebar_row_returns_focus_to_terminal", () => {
    (document.activeElement as HTMLElement | null)?.blur();
    row.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    // The rule waits for the click that follows the mouseup, so a row is selected first.
    expect(focusCurrent).not.toHaveBeenCalled();
    vi.runAllTimers();
    expect(focusCurrent).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(terminal);
  });

  it("open_rename_field_keeps_focus_until_enter_or_escape", () => {
    const field = document.createElement("input");
    row.append(field);
    holdFocus(field);
    expect(document.activeElement).toBe(field);
    row.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    root.dispatchEvent(new Event("drop", { bubbles: true }));
    refocus();
    vi.runAllTimers();
    expect(focusCurrent).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(field);
    // Other keys are typing in the field.
    press(field, "a");
    expect(document.activeElement).toBe(field);
    press(field, "Enter");
    expect(focusCurrent).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(terminal);
  });

  it("focus_returns_after_escape", () => {
    const field = document.createElement("input");
    row.append(field);
    holdFocus(field);
    press(field, "Escape");
    expect(document.activeElement).toBe(terminal);
    // Once let go, the field no longer holds focus against the rule.
    field.focus();
    row.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    vi.runAllTimers();
    expect(document.activeElement).toBe(terminal);
  });

  it("tree_change_and_window_focus_return_focus", () => {
    (document.activeElement as HTMLElement | null)?.blur();
    refocus();
    expect(focusCurrent).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event("focus"));
    vi.runAllTimers();
    expect(focusCurrent).toHaveBeenCalledTimes(2);
  });
});
