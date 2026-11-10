import { describe, expect, it, vi } from "vitest";

import type { Group, Tab, Tree } from "./connection";
import { refocus } from "./focus";
import { MAX_WIDTH, MIN_WIDTH, Sidebar, clampWidth, fitWidth } from "./sidebar";
import { Store } from "./store";
import css from "./styles.css?raw";

vi.mock("./focus", () => ({ refocus: vi.fn() }));

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
  const resized = vi.fn();
  const sidebar = new Sidebar(element, store, {
    select,
    collapse,
    focusPane: vi.fn(),
    title: () => undefined,
    resized,
  });
  sidebar.render();
  return { element, store, sidebar, select, collapse, resized };
}

function setWindowWidth(px: number): void {
  Object.defineProperty(window, "innerWidth", { value: px, configurable: true, writable: true });
}

function key(target: Element, name: string): void {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true }));
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

  it("width_leaves_the_terminal_at_least_320_px", () => {
    expect(fitWidth(240, 1280)).toBe(240);
    expect(fitWidth(480, 700)).toBe(380);
    expect(fitWidth(300, 488)).toBe(168);
    // Never below the minimum, even when the window is narrower than both together.
    expect(fitWidth(300, 400)).toBe(168);
    // A saved value outside 168 to 480 is clamped first.
    expect(fitWidth(900, 2000)).toBe(480);
    expect(fitWidth(50, 2000)).toBe(168);
    expect(fitWidth(Number.NaN, 2000)).toBe(240);
  });

  it("saved_width_comes_back_when_the_window_grows", async () => {
    setWindowWidth(600);
    const { element, sidebar, resized } = mount(tree([group("a", [tab("a1")])]));
    sidebar.setWidth(400);
    expect(element.style.width).toBe("280px");
    setWindowWidth(1280);
    window.dispatchEvent(new Event("resize"));
    window.dispatchEvent(new Event("resize"));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(element.style.width).toBe("400px");
    expect(resized).not.toHaveBeenCalled();
    setWindowWidth(1024);
  });

  it("edge_drag_clamps_saves_and_gives_typing_back", () => {
    vi.mocked(refocus).mockClear();
    const { element, sidebar, resized } = mount(tree([group("a", [tab("a1")])]));
    sidebar.setWidth(240);
    const grip = element.querySelector<HTMLElement>(".grip");
    if (!grip) throw new Error("no grip");
    grip.setPointerCapture = vi.fn();
    const at = (type: string, clientX: number) =>
      grip.dispatchEvent(new MouseEvent(type, { button: 0, clientX, bubbles: true, cancelable: true }));
    at("pointerdown", 240);
    at("pointermove", 400);
    at("pointermove", 900);
    expect(element.style.width).toBe("480px");
    at("pointerup", 900);
    expect(resized).toHaveBeenCalledTimes(1);
    expect(resized).toHaveBeenCalledWith(480);
    expect(refocus).toHaveBeenCalledTimes(1);
    // A cancelled drag gives typing back too.
    at("pointerdown", 480);
    at("pointercancel", 480);
    expect(refocus).toHaveBeenCalledTimes(2);
    expect(resized).toHaveBeenCalledTimes(1);
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

  it("group_count_sits_at_the_right_edge_without_a_rolled_up_marker", () => {
    const { element } = mount(tree([group("a", [tab("a1")]), group("b", [tab("b1")], true)]));
    for (const head of element.querySelectorAll(".ghead")) {
      expect(head.querySelector(".roll")).toBeNull();
      expect(head.lastElementChild?.classList.contains("gcount")).toBe(true);
    }
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

  it("rows_are_named_buttons_and_the_selected_one_is_current", () => {
    const { element } = mount(tree([group("a", [tab("a1", "first"), tab("a2", "second")])]));
    const rows = [...element.querySelectorAll(".row")];
    expect(rows.map((row) => row.getAttribute("role"))).toEqual(["button", "button"]);
    expect(rows.map((row) => row.getAttribute("aria-label"))).toEqual(["first", "second"]);
    expect(rows.map((row) => row.getAttribute("aria-current"))).toEqual(["true", null]);
  });

  it("one_tab_stop_and_up_and_down_move_between_rows_and_headers", () => {
    const { element } = mount(tree([group("a", [tab("a1"), tab("a2")])], [tab("u1")]));
    const items = [...element.querySelectorAll<HTMLElement>(".row, .ghead > .slot")];
    expect(items.map((item) => item.dataset.tab ?? "fold")).toEqual(["u1", "fold", "a1", "a2"]);
    const stops = items.filter((item) => item.tabIndex === 0);
    expect(stops).toEqual([element.querySelector(".row.selected")]);

    items[0].focus();
    key(items[0], "ArrowDown");
    expect(document.activeElement).toBe(items[1]);
    key(items[1], "ArrowDown");
    expect(document.activeElement).toBe(items[2]);
    key(items[2], "ArrowUp");
    expect(document.activeElement).toBe(items[1]);
    // The focused item becomes the one tab stop.
    expect(items.filter((item) => item.tabIndex === 0)).toEqual([items[1]]);
    // Stays put at either end.
    items[3].focus();
    key(items[3], "ArrowDown");
    expect(document.activeElement).toBe(items[3]);
  });

  it("enter_or_space_selects_a_tab_or_toggles_a_group_then_gives_typing_back", () => {
    vi.mocked(refocus).mockClear();
    const { element, select, collapse } = mount(tree([group("a", [tab("a1"), tab("a2")])]));
    const row = element.querySelector<HTMLElement>('[data-tab="a2"]');
    const fold = element.querySelector<HTMLElement>(".ghead > .slot");
    if (!row || !fold) throw new Error("no row");
    key(row, "Enter");
    expect(select).toHaveBeenCalledWith("a2");
    expect(refocus).toHaveBeenCalledTimes(1);
    key(row, " ");
    expect(select).toHaveBeenCalledTimes(2);
    key(fold, " ");
    expect(collapse).toHaveBeenCalledWith("a", true);
    expect(refocus).toHaveBeenCalledTimes(3);
  });

  it("no_sticky_headers", () => {
    // The real stylesheet, not an empty stand-in.
    expect(css).toContain("#sidebar {");
    expect(css).not.toMatch(/position\s*:\s*sticky/i);
  });
});
