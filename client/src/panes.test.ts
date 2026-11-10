import { describe, expect, it, vi } from "vitest";

import type { Split, Tab } from "./connection";
import { Panes } from "./panes";
import { startRename } from "./rename";
import type { TerminalView } from "./terminal";

type FakeView = { focus: ReturnType<typeof vi.fn>; setFontSize: ReturnType<typeof vi.fn> };

function leaf(session: string): Split {
  return { type: "leaf", session };
}

function row(...sessions: string[]): Split {
  return { type: "row", children: sessions.map(leaf), sizes: sessions.map(() => 1) };
}

function tab(id: string, split: Split): Tab {
  return { id, name: null, pinned: false, split, last_used: 0 };
}

function mount() {
  const root = document.createElement("div");
  document.body.append(root);
  const views = new Map<string, FakeView>();
  const boxes = new Map<string, HTMLElement>();
  const panes = new Panes(root, (container, session) => {
    const view = { focus: vi.fn(), setFontSize: vi.fn(), dispose: vi.fn(), setDimmed: vi.fn() };
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
});
