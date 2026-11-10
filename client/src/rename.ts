// The rename field over a row's name. Enter saves, Esc cancels, and typing goes back to the
// terminal either way.

import { holdFocus } from "./focus";

/** Fired on the row, bubbling, once the field is gone, whether it saved or not. */
export const RENAME_END = "renameend";

let open = 0;

/** True while a rename field is open, so pointing at a pane does not take its typing. */
export function renaming(): boolean {
  return open > 0;
}

/**
 * Puts a field over the name in `row` (a tab row's `.name` or a group header's `.gname`) with
 * `current` selected. Enter calls `done` with the new name, or null for an empty one, which
 * brings the automatic name back. Esc, or a click elsewhere in the app, calls nothing; switching
 * to another app and back keeps the field open. Enter with the text unchanged calls nothing too,
 * so an automatic name is never turned into a typed one.
 */
export function startRename(row: HTMLElement, current: string, done: (name: string | null) => void): void {
  const name = row.querySelector<HTMLElement>(".name, .gname");
  if (!name || row.querySelector("input.rename")) return;
  const field = document.createElement("input");
  field.className = "rename";
  field.type = "text";
  field.value = current;
  field.spellcheck = false;
  field.autocomplete = "off";
  field.setAttribute("aria-label", "Name");
  let finished = false;

  const finish = (save: boolean): void => {
    if (finished) return;
    finished = true;
    open--;
    const text = field.value.trim();
    field.replaceWith(name);
    if (save && text !== current) done(text === "" ? null : text);
    row.dispatchEvent(new Event(RENAME_END, { bubbles: true }));
  };

  // Added before holdFocus's own listener, so the field is read before focus moves away.
  field.addEventListener("keydown", (event) => {
    if (event.isComposing) return;
    if (event.key === "Enter") finish(true);
    else if (event.key === "Escape") finish(false);
  });
  field.addEventListener("blur", () => {
    // Switching to another app blurs the field too, while it stays the page's focused element.
    // The name stays open for when the window comes back.
    if (!document.hasFocus() || document.activeElement === field) return;
    finish(false);
  });
  // A click inside the field places the caret and is not a click on the row.
  field.addEventListener("click", (event) => event.stopPropagation());
  field.addEventListener("dblclick", (event) => event.stopPropagation());

  name.replaceWith(field);
  open++;
  holdFocus(field);
  field.select();
}
