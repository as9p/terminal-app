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

  it("after_a_rename_the_pane_row_under_the_pointer_takes_focus_again", () => {
    const split: Split = {
      type: "row",
      children: [{ type: "leaf", session: "p1" }, { type: "leaf", session: "p2" }],
      sizes: [1, 1],
    };
    const hoverPane = vi.fn();
    const { element, sidebar } = mount(tree([], [tab("t", null, split)]), { hoverPane });
    const paneRow = (session: string) => element.querySelector(`.pane-row[data-session="${session}"]`);
    sidebar.renameCurrent();
    paneRow("p2")?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    paneRow("p2")?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    expect(hoverPane).not.toHaveBeenCalled();
    element.querySelector("input.rename")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(hoverPane).not.toHaveBeenCalled();
    // The pointer has not left the row, so no mouseover comes; the next move over it is enough.
    paneRow("p2")?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    expect(hoverPane).toHaveBeenCalledTimes(1);
    expect(hoverPane).toHaveBeenCalledWith("t", "p2");
    // Later moves inside the same row change nothing.
    paneRow("p2")?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    expect(hoverPane).toHaveBeenCalledTimes(1);
  });

  it("leaving_the_list_lets_the_same_pane_row_take_focus_again", () => {
    const split: Split = {
      type: "row",
      children: [{ type: "leaf", session: "p1" }, { type: "leaf", session: "p2" }],
      sizes: [1, 1],
    };
    const hoverPane = vi.fn();
    const { element, sidebar } = mount(tree([], [tab("t", null, split)]), { hoverPane });
    const list = element.querySelector("#list");
    const over = (session: string) =>
      element.querySelector(`.pane-row[data-session="${session}"]`)?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    over("p1");
    expect(hoverPane).toHaveBeenCalledTimes(1);
    // Out to a terminal pane, which may take focus, and back to the same row.
    list?.dispatchEvent(new MouseEvent("pointerleave"));
    over("p1");
    expect(hoverPane).toHaveBeenCalledTimes(2);
    expect(hoverPane).toHaveBeenLastCalledWith("t", "p1");
    // Still nothing while a rename is open.
    sidebar.renameCurrent();
    list?.dispatchEvent(new MouseEvent("pointerleave"));
    over("p1");
    expect(hoverPane).toHaveBeenCalledTimes(2);
    element.querySelector("input.rename")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  });

  it("esc_on_a_focused_row_gives_typing_back", () => {
    vi.mocked(refocus).mockClear();
    const { element } = mount(tree([group("a", [tab("a1"), tab("a2")])]));
    const row = element.querySelector<HTMLElement>('[data-tab="a2"]');
    const fold = element.querySelector<HTMLElement>(".ghead > .slot");
    if (!row || !fold) throw new Error("no row");
    row.focus();
    const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    row.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(refocus).toHaveBeenCalledTimes(1);
    key(fold, "Escape");
    expect(refocus).toHaveBeenCalledTimes(2);
  });

  it("sub_line_trims_the_title_like_the_name", () => {
    const { element, store, sidebar } = mount(tree([], [tab("t")]), { title: () => "  build  " });
    store.setStates([{ session: "s-t", state: "blocked", since_ms: 1 }]);
    sidebar.updateStates();
    const row = element.querySelector<HTMLElement>('[data-tab="t"]');
    expect(row?.querySelector(".name")?.textContent).toBe("build");
    expect(row?.querySelector(".sub")?.textContent).toBe("build");
  });

  describe("tab switch", () => {
    const split = (a: string, b: string): Split => ({
      type: "row",
      children: [{ type: "leaf", session: a }, { type: "leaf", session: b }],
      sizes: [1, 1],
    });

    it("a_tab_switch_updates_in_place_and_keeps_unaffected_rows_and_markers", () => {
      const t = tree([group("a", [tab("a1"), tab("a2"), tab("a3")]), group("b", [tab("b1")])], [tab("u1")]);
      const { element, store, sidebar } = mount(t);
      store.setStates([
        { session: "s-a1", state: "working", since_ms: 1 },
        { session: "s-a2", state: "blocked", since_ms: 1 },
        { session: "s-a3", state: "done", since_ms: 1 },
        { session: "s-b1", state: "working", since_ms: 1 },
      ]);
      sidebar.updateStates();
      const rows = [...element.querySelectorAll<HTMLElement>(".row, .ghead")];
      const markers = [...element.querySelectorAll<HTMLElement>(".mk")];
      expect(markers.length).toBe(4);
      const before = element.querySelector<HTMLElement>('[data-tab="u1"]');
      expect(before?.classList.contains("selected")).toBe(true);

      store.select("a2");
      sidebar.render();

      expect([...element.querySelectorAll<HTMLElement>(".row, .ghead")]).toEqual(rows);
      // Every marker is the node it was, including the two switched rows', so none restarts a morph.
      expect([...element.querySelectorAll<HTMLElement>(".mk")]).toEqual(markers);
      const now = element.querySelector<HTMLElement>('[data-tab="a2"]');
      expect(before?.classList.contains("selected")).toBe(false);
      expect(before?.getAttribute("aria-current")).toBeNull();
      expect(before?.tabIndex).toBe(-1);
      expect(now?.classList.contains("selected")).toBe(true);
      expect(now?.getAttribute("aria-current")).toBe("true");
      expect(now?.tabIndex).toBe(0);
      expect([...element.querySelectorAll<HTMLElement>(".row, .ghead > .slot")].filter((i) => i.tabIndex === 0)).toEqual([now]);
    });

    it("a_tab_switch_rebuilds_only_the_pane_rows_of_the_two_tabs", () => {
      const t = tree([group("a", [tab("a1", null, split("p1", "p2")), tab("a2"), tab("a3", null, split("q1", "q2"))])]);
      const { element, store, sidebar } = mount(t);
      store.select("a1");
      sidebar.render();
      const rows = [...element.querySelectorAll<HTMLElement>(".row, .ghead")];
      expect([...element.querySelectorAll<HTMLElement>(".pane-row")].map((r) => r.dataset.session)).toEqual(["p1", "p2"]);

      store.select("a3");
      sidebar.render();
      expect([...element.querySelectorAll<HTMLElement>(".row, .ghead")]).toEqual(rows);
      const panes = [...element.querySelectorAll<HTMLElement>(".pane-row")];
      expect(panes.map((r) => r.dataset.session)).toEqual(["q1", "q2"]);
      expect(panes.map((r) => r.querySelector(".pidx")?.textContent)).toEqual(["1", "2"]);
      // The panes sit right under their tab's row.
      expect(element.querySelector('[data-tab="a3"]')?.nextElementSibling?.classList.contains("panes")).toBe(true);
      expect(element.querySelectorAll(".panes").length).toBe(1);

      store.select("a2");
      sidebar.render();
      expect(element.querySelectorAll(".panes, .pane-row").length).toBe(0);
      expect([...element.querySelectorAll<HTMLElement>(".row, .ghead")]).toEqual(rows);
    });
  });

  describe("context menu keys", () => {
    function menuSetup() {
      const op = vi.fn();
      const { element } = mount(tree([group("a", [tab("a1")]), group("b", [tab("b1")])], [tab("u1")]), { op });
      const row = element.querySelector<HTMLElement>('[data-tab="a1"]');
      if (!row) throw new Error("no row");
      row.focus();
      const terminal = document.createElement("textarea");
      document.body.append(terminal);
      const typed = vi.fn();
      terminal.addEventListener("keydown", typed);
      const press = (name: string, init: KeyboardEventInit = {}) => {
        const event = new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...init });
        (document.activeElement ?? document.body).dispatchEvent(event);
        return event;
      };
      const items = () => [...document.querySelectorAll<HTMLElement>(".ctx-menu .item")];
      const label = () => document.activeElement?.querySelector(".label")?.textContent;
      const done = () => {
        press("Escape");
        press("Escape");
        terminal.remove();
      };
      return { element, op, row, press, items, label, typed, done };
    }

    it("shift_f10_opens_the_menu_with_focus_on_the_first_item", () => {
      const { press, items, label, done } = menuSetup();
      press("F10", { shiftKey: true });
      expect(document.querySelector(".ctx-menu")?.getAttribute("role")).toBe("menu");
      expect(items().map((item) => item.getAttribute("role"))).toEqual(["menuitem", "menuitem", "menuitem", "menuitem"]);
      expect(label()).toBe("Rename");
      done();
    });

    it("arrows_wrap_home_and_end_jump_and_esc_returns_to_the_row", () => {
      const { row, press, label, typed, done } = menuSetup();
      press("ContextMenu");
      expect(label()).toBe("Rename");
      press("ArrowDown");
      expect(label()).toBe("Pin");
      press("ArrowUp");
      press("ArrowUp");
      expect(label()).toBe("Move to group");
      press("ArrowDown");
      expect(label()).toBe("Rename");
      press("End");
      expect(label()).toBe("Move to group");
      press("Home");
      expect(label()).toBe("Rename");
      const esc = press("Escape");
      expect(esc.defaultPrevented).toBe(true);
      expect(document.querySelector(".ctx-menu")).toBeNull();
      expect(document.activeElement).toBe(row);
      expect(row.classList.contains("menu-open")).toBe(false);
      expect(typed).not.toHaveBeenCalled();
      done();
    });

    it("enter_activates_an_item_and_gives_typing_back", () => {
      vi.mocked(refocus).mockClear();
      const { op, press, done } = menuSetup();
      press("F10", { shiftKey: true });
      press("ArrowDown");
      press("Enter");
      expect(op).toHaveBeenCalledWith({ type: "pin_tab", tab: "a1", pinned: true });
      expect(document.querySelector(".ctx-menu")).toBeNull();
      expect(refocus).toHaveBeenCalledTimes(1);
      done();
    });

    it("space_or_right_opens_the_submenu_and_left_comes_back", () => {
      const { op, press, label, done } = menuSetup();
      press("F10", { shiftKey: true });
      press("End");
      press("ArrowRight");
      expect(document.querySelectorAll(".ctx-menu").length).toBe(2);
      expect(label()).toBe("No group");
      press("ArrowLeft");
      expect(document.querySelectorAll(".ctx-menu").length).toBe(1);
      expect(label()).toBe("Move to group");
      press(" ");
      expect(label()).toBe("No group");
      // Esc in the submenu closes only the submenu.
      press("Escape");
      expect(document.querySelectorAll(".ctx-menu").length).toBe(1);
      expect(label()).toBe("Move to group");
      press(" ");
      press("ArrowDown");
      expect(label()).toBe("b");
      press("Enter");
      expect(op).toHaveBeenCalledWith({ type: "move_tab", tab: "a1", group: "b", index: 1 });
      expect(document.querySelector(".ctx-menu")).toBeNull();
      done();
    });

    it("menu_items_show_focus_with_the_hover_fill", () => {
      expect(css).toMatch(/\.ctx-menu \.item:not\(\.disabled\):hover,\s*\.ctx-menu \.item:not\(\.disabled\):focus-visible \{/);
    });
  });

  describe("row drag", () => {
    function dragSetup(listBottom = 200) {
      const op = vi.fn();
      const { element, select } = mount(tree([], [tab("a"), tab("b"), tab("c")]), { op });
      const list = element.querySelector<HTMLElement>("#list");
      if (!list) throw new Error("no list");
      list.getBoundingClientRect = () =>
        ({ top: 0, bottom: listBottom, left: 0, right: 200, width: 200, height: listBottom, x: 0, y: 0 }) as DOMRect;
      // How far the list has scrolled, which moves every row up.
      const scroll = { by: 0 };
      element.querySelectorAll<HTMLElement>(".row").forEach((row, i) => {
        row.getBoundingClientRect = () => {
          const top = i * 26 - scroll.by;
          return { top, bottom: top + 26, left: 0, right: 200, width: 200, height: 26, x: 0, y: top } as DOMRect;
        };
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
      return { element, list, scroll, op, select, rowA, pointer, esc, typed, terminal };
    }

    it("a_drag_drops_on_release", () => {
      const { op, pointer, terminal } = dragSetup();
      pointer("pointerup", 39);
      expect(op).toHaveBeenCalledWith({ type: "join_tabs", tab: "a", into: "b", dir: "right" });
      terminal.remove();
    });

    it("a_scroll_during_a_drag_measures_the_rows_again", async () => {
      const { element, list, scroll, op, pointer, terminal } = dragSetup();
      expect(element.querySelector('[data-tab="b"]')?.classList.contains("join-target")).toBe(true);
      // The wheel moves the list a row up: c is now under the pointer.
      scroll.by = 26;
      list.dispatchEvent(new Event("scroll"));
      list.dispatchEvent(new Event("scroll"));
      await new Promise((resolve) => requestAnimationFrame(resolve));
      expect(element.querySelector('[data-tab="c"]')?.classList.contains("join-target")).toBe(true);
      expect(element.querySelector('[data-tab="b"]')?.classList.contains("join-target")).toBe(false);
      pointer("pointerup", 39);
      expect(op).toHaveBeenCalledWith({ type: "join_tabs", tab: "a", into: "c", dir: "right" });
      terminal.remove();
    });

    it("the_pointer_past_the_list_hits_the_last_visible_row_not_a_clipped_one", () => {
      // Two rows show; c is scrolled out below.
      const { op, pointer, terminal } = dragSetup(52);
      pointer("pointermove", 65);
      pointer("pointerup", 65);
      expect(op).toHaveBeenCalledWith({ type: "move_tab", tab: "a", group: null, index: 1 });
      terminal.remove();
    });

    it("the_ghost_is_measured_once_per_drag", () => {
      let reads = 0;
      const own = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
      Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
        configurable: true,
        get(this: HTMLElement) {
          if (this.classList.contains("ghost")) reads++;
          return 26;
        },
      });
      try {
        const { pointer, terminal } = dragSetup();
        pointer("pointermove", 45);
        pointer("pointermove", 50);
        pointer("pointermove", 60);
        expect(reads).toBe(1);
        pointer("pointerup", 60);
        terminal.remove();
      } finally {
        if (own) Object.defineProperty(HTMLElement.prototype, "offsetHeight", own);
      }
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
