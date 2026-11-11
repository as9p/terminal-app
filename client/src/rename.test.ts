import { afterEach, describe, expect, it, vi } from "vitest";

import { renaming, startRename } from "./rename";

function rowWithName(text: string): HTMLElement {
  const row = document.createElement("div");
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = text;
  row.append(name);
  document.body.append(row);
  return row;
}

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("rename", () => {
  it("the_field_asks_for_no_system_suggestions", () => {
    const row = rowWithName("old");
    startRename(row, "old", vi.fn());
    const field = row.querySelector<HTMLInputElement>("input.rename");
    if (!field) throw new Error("no field");
    expect(field.getAttribute("autocorrect")).toBe("off");
    expect(field.getAttribute("autocapitalize")).toBe("off");
    expect(field.getAttribute("writingsuggestions")).toBe("false");
    expect(field.spellcheck).toBe(false);
    expect(field.autocomplete).toBe("off");
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  });

  it("switching_apps_keeps_the_rename_open", () => {
    const row = rowWithName("old");
    const done = vi.fn();
    startRename(row, "old", done);
    const field = row.querySelector<HTMLInputElement>("input.rename");
    if (!field) throw new Error("no field");
    // The window loses focus: the field blurs while the page has no focus.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    field.dispatchEvent(new FocusEvent("blur"));
    expect(row.querySelector("input.rename")).toBe(field);
    expect(renaming()).toBe(true);
    // Back in the window, Enter still saves.
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    field.value = "new";
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(done).toHaveBeenCalledWith("new");
    expect(row.querySelector("input.rename")).toBeNull();
    expect(renaming()).toBe(false);
  });

  it("a_click_elsewhere_in_the_app_still_cancels", () => {
    const row = rowWithName("old");
    const done = vi.fn();
    startRename(row, "old", done);
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const field = row.querySelector<HTMLInputElement>("input.rename");
    field?.blur();
    expect(row.querySelector("input.rename")).toBeNull();
    expect(done).not.toHaveBeenCalled();
    expect(renaming()).toBe(false);
  });

  it("esc_cancels", () => {
    const row = rowWithName("old");
    const done = vi.fn();
    startRename(row, "old", done);
    expect(renaming()).toBe(true);
    row.querySelector("input")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(done).not.toHaveBeenCalled();
    expect(renaming()).toBe(false);
  });
});
