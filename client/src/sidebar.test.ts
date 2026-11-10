import { describe, expect, it, vi } from "vitest";

import type { Group, Split, Tab, Tree } from "./connection";
import { refocus } from "./focus";
import { MAX_WIDTH, MIN_WIDTH, Sidebar, type SidebarEvents, clampWidth, fitWidth } from "./sidebar";
import { Store } from "./store";
import css from "./styles.css?raw";

vi.mock("./focus", () => ({ refocus: vi.fn(), holdFocus: (field: HTMLElement) => field.focus() }));

function tab(id: string, name: string | null = null, split: Split = { type: "leaf", session: `s-${id}` }): Tab {
  return { id, name, pinned: false, split, last_used: 0 };
}

function group(key: string, tabs: Tab[], collapsed = false): Group {
  return { key, path: `/code/${key}`, name: null, branch: "main", collapsed, tabs };
}

function tree(groups: Group[], ungrouped: Tab[] = []): Tree {
  return { groups, ungrouped, closed: [], defaults: {} };
}

function mount(t: Tree, extra: Partial<SidebarEvents> = {}) {
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
    ...extra,
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

  it("double_click_on_a_tab_row_sends_used_once", () => {
    let store: Store | undefined;
    // As main.ts does: selecting makes the tab current.
    const select = vi.fn((id: string) => store?.select(id));
    const mounted = mount(tree([], [tab("a"), tab("b")]), { select });
    store = mounted.store;
    const { element } = mounted;
    const row = element.querySelector<HTMLElement>('[data-tab="b"]');
    row?.click();
    row?.click();
    expect(select).toHaveBeenCalledTimes(1);
    expect(select).toHaveBeenCalledWith("b");
  });

  it("pointing_at_a_pane_row_does_not_take_focus_while_a_rename_is_open", () => {
    const split: Split = {
      type: "row",
      children: [{ type: "leaf", session: "p1" }, { type: "leaf", session: "p2" }],
      sizes: [1, 1],
    };
    const hoverPane = vi.fn();
    const { element, sidebar } = mount(tree([], [tab("t", null, split)]), { hoverPane });
    const over = (session: string) =>
      element.querySelector(`.pane-row[data-session="${session}"]`)?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    over("p1");
    expect(hoverPane).toHaveBeenCalledTimes(1);
    sidebar.renameCurrent();
    expect(element.querySelector("input.rename")).not.toBeNull();
    over("p2");
    expect(hoverPane).toHaveBeenCalledTimes(1);
    element.querySelector("input.rename")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  });

  describe("row drag", () => {
    function dragSetup() {
      const op = vi.fn();
      const { element, select } = mount(tree([], [tab("a"), tab("b"), tab("c")]), { op });
      element.querySelectorAll<HTMLElement>(".row").forEach((row, i) => {
        row.getBoundingClientRect = () =>
          ({ top: i * 26, bottom: (i + 1) * 26, left: 0, right: 200, width: 200, height: 26, x: 0, y: i * 26 }) as DOMRect;
      });
      const rowA = element.querySelector<HTMLElement>('[data-tab="a"]');
      if (!rowA) throw new Error("no row");
      const pointer = (type: string, clientY: number, target: EventTarget = window) =>
        target.dispatchEvent(new MouseEvent(type, { button: 0, clientX: 20, clientY, bubbles: true, cancelable: true }));
      // Onto the middle of row b: a join.
      pointer("pointerdown", 5, rowA);
      pointer("pointermove", 39);
      const terminal = document.createElement("textarea");
      document.body.append(terminal);
      const typed = vi.fn();
      terminal.addEventListener("keydown", typed);
      const esc = () => {
        const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
        terminal.dispatchEvent(event);
        return event;
      };
      return { element, op, select, rowA, pointer, esc, typed, terminal };
    }

    it("a_drag_drops_on_release", () => {
      const { op, pointer, terminal } = dragSetup();
      pointer("pointerup", 39);
      expect(op).toHaveBeenCalledWith({ type: "join_tabs", tab: "a", into: "b", dir: "right" });
      terminal.remove();
    });

    it("esc_ends_a_drag_with_no_drop_and_never_reaches_the_terminal", () => {
      const { element, op, select, rowA, pointer, esc, typed, terminal } = dragSetup();
      expect(element.querySelector(".drag-src")).not.toBeNull();
      const event = esc();
      expect(event.defaultPrevented).toBe(true);
      expect(typed).not.toHaveBeenCalled();
      expect(element.querySelector(".drag-src, .join")).toBeNull();
      expect(element.querySelector(".ghost")).toBeNull();
      // Moving on and letting go drops nothing, and the click that follows is not a row click.
      pointer("pointermove", 60);
      pointer("pointerup", 60);
      rowA.click();
      expect(op).not.toHaveBeenCalled();
      expect(select).not.toHaveBeenCalled();
      // The Esc listener lived only as long as the drag.
      expect(esc().defaultPrevented).toBe(false);
      expect(typed).toHaveBeenCalledTimes(1);
      terminal.remove();
    });
  });

  it("no_sticky_headers", () => {
    // The real stylesheet, not an empty stand-in.
    expect(css).toContain("#sidebar {");
    expect(css).not.toMatch(/position\s*:\s*sticky/i);
  });
});
