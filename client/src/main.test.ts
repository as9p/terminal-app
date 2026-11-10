import { describe, expect, it } from "vitest";

import { focusHeldAside } from "./main";

describe("refocus after a tree change", () => {
  it("an_open_menu_or_the_sidebar_keeps_focus", () => {
    const sidebar = document.createElement("div");
    const row = document.createElement("button");
    sidebar.append(row);
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    const item = document.createElement("button");
    item.setAttribute("role", "menuitem");
    menu.append(item);
    const pane = document.createElement("textarea");
    document.body.append(sidebar, menu, pane);
    item.focus();
    expect(focusHeldAside(sidebar)).toBe(true);
    row.focus();
    expect(focusHeldAside(sidebar)).toBe(true);
    pane.focus();
    expect(focusHeldAside(sidebar)).toBe(false);
    // The menu closed: focus on the page goes back to the terminal.
    menu.remove();
    (document.activeElement as HTMLElement | null)?.blur();
    expect(focusHeldAside(sidebar)).toBe(false);
    sidebar.remove();
    pane.remove();
  });
});
