import { describe, expect, it, vi } from "vitest";

import type { Split, Tab, Tree } from "./connection";
import { type ActionContext, runAction } from "./keys";
import { Panes, WEBGL_CAP, webglEvictions } from "./panes";
import { startRename } from "./rename";
import { Store, allTabs, leaves } from "./store";
import type { TerminalView } from "./terminal";

// `webgl` stands in for the view's WebGL addon: true while it holds a live context.
type FakeView = { focus: ReturnType<typeof vi.fn>; setFontSize: ReturnType<typeof vi.fn>; webgl: boolean };

function leaf(session: string): Split {
  return { type: "leaf", session };
}

function row(...sessions: string[]): Split {
  return { type: "row", children: sessions.map(leaf), sizes: sessions.map(() => 1) };
}

function tab(id: string, split: Split): Tab {
  return { id, name: null, pinned: false, split, last_used: 0 };
}

/** With `realFocus`, a view taking focus fires focusin on its pane, as a real terminal does. */
function mount(realFocus = false) {
  const root = document.createElement("div");
  document.body.append(root);
  const views = new Map<string, FakeView>();
  const boxes = new Map<string, HTMLElement>();
  const panes = new Panes(root, (container, session) => {
    const view = {
      focus: vi.fn(() => {
        if (realFocus) container.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      }),
      setFontSize: vi.fn(),
      dispose: vi.fn(() => {
        view.webgl = false;
      }),
      setDimmed: vi.fn(),
      // A new view loads its context, as TerminalView does.
      webgl: true,
      webglOn: () => view.webgl,
      setWebgl: (on: boolean) => {
        view.webgl = on;
      },
    };
    views.set(session, view);
    boxes.set(session, container);
    return view as unknown as TerminalView;
  });
  // What a click in a pane does: its terminal takes focus, and the pane hears focusin.
  const focusIn = (session: string) => boxes.get(session)?.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
  return { root, panes, views, boxes, focusIn };
}

describe("panes", () => {
  it("closing_the_right_pane_of_three_focuses_its_left_neighbor", () => {
    const { root, panes, focusIn } = mount();
    panes.show(tab("t", row("a", "b", "c")));
    focusIn("c");
    expect(panes.focusedSession()).toBe("c");
    panes.show(tab("t", row("a", "b")));
    expect(panes.focusedSession()).toBe("b");
    root.remove();
  });

  it("closing_the_first_pane_focuses_the_next", () => {
    const { root, panes, focusIn } = mount();
    panes.show(tab("t", row("a", "b", "c")));
    focusIn("a");
    panes.show(tab("t", row("b", "c")));
    expect(panes.focusedSession()).toBe("b");
    root.remove();
  });

  it("prune_forgets_focus_on_sessions_that_are_gone", () => {
    const { root, panes, focusIn } = mount();
    panes.show(tab("t1", row("a", "b")));
    focusIn("b");
    panes.show(tab("t2", leaf("c")));
    panes.prune(new Set(["a", "c"]));
    const focused = (panes as unknown as { focused: Map<string, string> }).focused;
    expect(focused.has("t1")).toBe(false);
    root.remove();
  });

  it("closing_the_focused_pane_of_a_background_tab_remembers_its_neighbor", () => {
    const { root, panes, focusIn, views } = mount();
    // As main.ts does on each tree: the current tab is shown, then every tab is passed to prune.
    panes.show(tab("t1", row("a", "b", "c")));
    panes.prune(new Set(["a", "b", "c", "d"]), [tab("t1", row("a", "b", "c")), tab("t2", leaf("d"))]);
    focusIn("c");
    panes.show(tab("t2", leaf("d")));
    panes.prune(new Set(["a", "b", "c", "d"]), [tab("t1", row("a", "b", "c")), tab("t2", leaf("d"))]);
    // "c" closes while t1 is in the background.
    panes.prune(new Set(["a", "b", "d"]), [tab("t1", row("a", "b")), tab("t2", leaf("d"))]);
    // Nothing on screen moved.
    expect(views.get("b")?.focus).not.toHaveBeenCalled();
    expect(panes.focusedSession()).toBe("d");
    panes.show(tab("t1", row("a", "b")));
    expect(panes.focusedSession()).toBe("b");
    root.remove();
  });

  it("leaving_fill_brings_the_split_back_so_a_new_pane_is_built", () => {
    const { root, panes, views } = mount();
    panes.show(tab("t", row("a", "b")));
    panes.toggleFill();
    expect(root.querySelector(".split")).toBeNull();
    panes.leaveFill();
    expect(root.querySelector(".split")).not.toBeNull();
    panes.show(tab("t", row("a", "b", "c")));
    expect(views.has("c")).toBe(true);
    // Not filled, leaving fill changes nothing.
    panes.leaveFill();
    expect(root.querySelectorAll(".split > .pane").length).toBe(3);
    root.remove();
  });

  it("pointing_at_a_pane_does_not_take_focus_while_a_rename_is_open", () => {
    const { root, panes, views, boxes } = mount();
    panes.setFocusFollowsMouse(true);
    panes.show(tab("t", row("a", "b", "c")));
    boxes.get("b")?.dispatchEvent(new MouseEvent("mouseenter"));
    expect(views.get("b")?.focus).toHaveBeenCalledTimes(1);
    const sideRow = document.createElement("div");
    const name = document.createElement("span");
    name.className = "name";
    sideRow.append(name);
    document.body.append(sideRow);
    startRename(sideRow, "old", () => {});
    boxes.get("c")?.dispatchEvent(new MouseEvent("mouseenter"));
    expect(views.get("c")?.focus).not.toHaveBeenCalled();
    sideRow.querySelector("input")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    boxes.get("c")?.dispatchEvent(new MouseEvent("mouseenter"));
    expect(views.get("c")?.focus).toHaveBeenCalledTimes(1);
    sideRow.remove();
    root.remove();
  });

  it("closing_the_focused_pane_of_a_background_tab_as_it_becomes_current_focuses_its_neighbor", () => {
    const { root, panes, focusIn } = mount();
    const all = new Set(["a", "b", "c", "d"]);
    panes.show(tab("t1", row("a", "b", "c")));
    panes.prune(all, [tab("t1", row("a", "b", "c")), tab("t2", leaf("d"))]);
    focusIn("c");
    panes.show(tab("t2", leaf("d")));
    panes.prune(all, [tab("t1", row("a", "b", "c")), tab("t2", leaf("d"))]);
    // One tree: "c" closes and t1 becomes the current tab.
    panes.show(tab("t1", row("a", "b")));
    panes.prune(new Set(["a", "b", "d"]), [tab("t1", row("a", "b")), tab("t2", leaf("d"))]);
    expect(panes.focusedSession()).toBe("b");
    root.remove();
  });
});

describe("webgl contexts", () => {
  it("evicts_the_least_recently_shown_parked_views_past_the_cap", () => {
    const all = new Set(["a", "b", "c", "d"]);
    expect(webglEvictions(["a", "b", "c", "d"], all, new Set(["d"]), 2)).toEqual(["a", "b"]);
    expect(webglEvictions(["a", "b", "c", "d"], all, new Set(["d"]), 4)).toEqual([]);
    // A visible view keeps its context even past the cap.
    expect(webglEvictions(["a", "b", "c"], new Set(["a", "b", "c"]), new Set(["a", "b", "c"]), 2)).toEqual([]);
    expect(webglEvictions(["a", "b", "c"], new Set(["a", "b", "c"]), new Set(["a", "c"]), 1)).toEqual(["b"]);
    // A view without a context has nothing to give up.
    expect(webglEvictions(["a", "b", "c"], new Set(["b", "c"]), new Set(["c"]), 1)).toEqual(["b"]);
  });

  it("visible_panes_always_hold_a_context_and_the_total_stays_within_the_cap", () => {
    const { root, panes, views } = mount();
    const live = () => [...views.values()].filter((view) => view.webgl).length;
    const check = (visible: string[]) => {
      for (const session of visible) expect(views.get(session)?.webgl).toBe(true);
      expect(live()).toBeLessThanOrEqual(Math.max(WEBGL_CAP, visible.length));
    };
    const count = WEBGL_CAP + 4;
    for (let i = 0; i < count; i++) {
      panes.show(tab(`t${i}`, leaf(`s${i}`)));
      check([`s${i}`]);
    }
    expect(live()).toBe(WEBGL_CAP);
    // The least recently shown went first, and one shown again gets its context back.
    expect(views.get("s0")?.webgl).toBe(false);
    expect(views.get(`s${count - WEBGL_CAP}`)?.webgl).toBe(true);
    panes.show(tab("t0", leaf("s0")));
    check(["s0"]);
    expect(views.get(`s${count - WEBGL_CAP}`)?.webgl).toBe(false);
    // More visible panes than the cap: each still gets one, and every parked view gives way.
    const many = Array.from({ length: WEBGL_CAP + 2 }, (_, i) => `m${i}`);
    panes.show(tab("big", row(...many)));
    check(many);
    expect(live()).toBe(many.length);
    panes.show(tab("t1", leaf("s1")));
    check(["s1"]);
    expect(live()).toBe(WEBGL_CAP);
    // Filling the tab parks the other panes, but they lose nothing until the cap is passed.
    panes.show(tab("t2", row("s2", "s3")));
    panes.toggleFill();
    check(["s2"]);
    expect(views.get("s3")?.webgl).toBe(true);
    root.remove();
  });
});

function tree(tabs: Tab[]): Tree {
  return { groups: [], ungrouped: tabs, closed: [], defaults: {} };
}

describe("split from a filled pane", () => {
  // main.ts shows the tree in one store listener; openShell follows the new pane in another.
  for (const followFirst of [false, true]) {
    it(`ends_with_the_new_pane_built_visible_and_focused_${followFirst ? "follow_first" : "panes_first"}`, async () => {
      const { root, panes, views, boxes } = mount(true);
      const store = new Store();
      const watch = () => {
        panes.show(store.currentTab());
        panes.prune(new Set(allTabs(store.tree).flatMap((t) => leaves(t.split))), allTabs(store.tree));
        panes.focusCurrent();
      };
      if (!followFirst) store.subscribe(watch);
      store.setTree(tree([tab("a", row("s-a", "s-b"))]));
      if (followFirst) watch();
      panes.focusSession("s-a");
      panes.toggleFill();
      expect(root.querySelector(".split")).toBeNull();
      const ctx: ActionContext = {
        store,
        panes,
        connection: { treeOp: vi.fn(), list: vi.fn(async () => []), create: vi.fn(async () => 1), kill: vi.fn() },
        home: "/home",
        ui: { toggleSidebar: vi.fn(), rename: vi.fn(), zoom: vi.fn(), note: vi.fn() },
      };
      const uuid = vi.spyOn(crypto, "randomUUID").mockReturnValue("0-0-0-0-new");
      runAction("splitRight", ctx);
      await vi.waitFor(() => expect(ctx.connection.create).toHaveBeenCalled());
      if (followFirst) store.subscribe(watch);
      store.setTree(tree([tab("a", row("s-a", "0-0-0-0-new", "s-b"))]));
      const box = boxes.get("0-0-0-0-new");
      expect(views.has("0-0-0-0-new")).toBe(true);
      expect(box?.isConnected).toBe(true);
      expect(box?.closest(".parked")).toBeNull();
      expect(root.querySelectorAll(".split > .pane").length).toBe(3);
      expect(views.get("0-0-0-0-new")?.focus).toHaveBeenCalled();
      expect(panes.focusedSession()).toBe("0-0-0-0-new");
      uuid.mockRestore();
      root.remove();
    });
  }
});
