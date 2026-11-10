import { describe, expect, it, vi } from "vitest";

import type { Group, Tab, Tree } from "./connection";
import { MAX_WIDTH, MIN_WIDTH, Sidebar, clampWidth } from "./sidebar";
import { Store } from "./store";

function tab(id: string, name: string | null = null): Tab {
  return { id, name, pinned: false, split: { type: "leaf", session: `s-${id}` }, last_used: 0 };
}

function group(key: string, tabs: Tab[], collapsed = false): Group {
  return { key, path: `/code/${key}`, name: null, branch: "main", collapsed, tabs };
}

function tree(groups: Group[], ungrouped: Tab[] = []): Tree {
  return { groups, ungrouped, closed: [], defaults: {} };
}

function mount(t: Tree) {
  const element = document.createElement("aside");
  document.body.append(element);
  const store = new Store();
  store.setTree(t);
  const select = vi.fn();
  const collapse = vi.fn();
  const sidebar = new Sidebar(element, store, {
    select,
    collapse,
    focusPane: vi.fn(),
    title: () => undefined,
  });
  sidebar.render();
  return { element, store, select, collapse };
}

describe("sidebar", () => {
  it("width_clamps_between_168_and_480", () => {
    expect(MIN_WIDTH).toBe(168);
    expect(MAX_WIDTH).toBe(480);
    expect(clampWidth(100)).toBe(168);
    expect(clampWidth(300.4)).toBe(300);
    expect(clampWidth(900)).toBe(480);
    expect(clampWidth(Number.NaN)).toBe(240);
  });

  it("ungrouped_rows_come_first_with_no_header", () => {
    const { element } = mount(tree([group("a", [tab("a1")])], [tab("u1", "loose one")]));
    const list = element.querySelector("#list");
    const sections = [...(list?.children ?? [])].map((child) => child.className);
    // The last child is the empty space that moves the window.
    expect(sections).toEqual(["loose", "group", "fill"]);
    expect(element.querySelector(".loose .name")?.textContent).toBe("loose one");
  });

  it("group_header_shows_folder_branch_and_session_count", () => {
    const { element } = mount(tree([group("a", [tab("a1"), tab("a2")])]));
    const head = element.querySelector(".ghead");
    expect(head?.querySelector(".gname")?.textContent).toBe("a");
    expect(head?.querySelector(".branch")?.textContent).toBe("main");
    expect(head?.querySelector(".gcount")?.textContent).toBe("2");
    const fold = head?.querySelector("button.slot");
    expect(fold?.getAttribute("aria-expanded")).toBe("true");
    expect(fold?.getAttribute("aria-label")).toBe("Collapse a");
  });

  it("row_click_selects_and_folder_click_collapses", () => {
    const { element, select, collapse } = mount(tree([group("a", [tab("a1"), tab("a2")])]));
    element.querySelector<HTMLElement>('[data-tab="a2"]')?.click();
    expect(select).toHaveBeenCalledWith("a2");
    element.querySelector<HTMLElement>(".ghead button.slot")?.click();
    expect(collapse).toHaveBeenCalledWith("a", true);
  });

  it("selected_row_is_marked_and_every_row_keeps_its_marker_slot", () => {
    const { element } = mount(tree([group("a", [tab("a1"), tab("a2")])]));
    expect(element.querySelector(".row.selected")?.getAttribute("data-tab")).toBe("a1");
    for (const row of element.querySelectorAll(".row")) {
      expect(row.firstElementChild?.classList.contains("slot")).toBe(true);
    }
  });

  it("no_sticky_headers", () => {
    const { element } = mount(tree([group("a", [tab("a1")])]));
    expect(element.innerHTML).not.toContain("sticky");
  });
});
