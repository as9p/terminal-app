import { describe, expect, it, vi } from "vitest";

import type { Group, Split, Tab, Tree } from "./connection";
import {
  MIN_PANE_COLS,
  MIN_PANE_ROWS,
  Store,
  clampPair,
  leaves,
  neighborOf,
  nextCurrent,
  paneFloor,
  roundToCells,
  tabAt,
  withCollapsed,
  withSizesAt,
} from "./store";

function leaf(session: string): Split {
  return { type: "leaf", session };
}

function tab(id: string, split: Split = leaf(`s-${id}`)): Tab {
  return { id, name: null, pinned: false, split, last_used: 0 };
}

function group(key: string, tabs: Tab[], collapsed = false): Group {
  return { key, path: `/code/${key}`, name: null, branch: "main", collapsed, tabs };
}

function tree(groups: Group[], ungrouped: Tab[] = []): Tree {
  return { groups, ungrouped, closed: [], defaults: {} };
}

/** A short text per row, so a whole list compares in one line. */
function shape(store: Store): string[] {
  return store.rows().map((row) => {
    switch (row.kind) {
      case "group":
        return `group ${row.group.key}`;
      case "tab":
        return `tab ${row.tab.id} ${row.depth}`;
      case "pane":
        return `pane ${row.session} of ${row.tab}`;
    }
  });
}

describe("store", () => {
  // docs/ui-review.md puts ungrouped tabs first with no header, so they lead the list.
  it("rows_put_ungrouped_then_group_then_its_tabs", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1"), tab("a2")]), group("b", [tab("b1")])], [tab("u1")]));
    expect(shape(store)).toEqual([
      "tab u1 0",
      "group a",
      "tab a1 1",
      "tab a2 1",
      "group b",
      "tab b1 1",
    ]);
  });

  it("collapsed_group_hides_its_tabs", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1")], true), group("b", [tab("b1")])]));
    expect(shape(store)).toEqual(["group a", "group b", "tab b1 1"]);
  });

  it("split_tab_lists_its_panes_under_it", () => {
    const split: Split = {
      type: "row",
      sizes: [0.5, 0.5],
      children: [leaf("left"), { type: "col", sizes: [0.5, 0.5], children: [leaf("top"), leaf("bottom")] }],
    };
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1", split), tab("a2", split)])]));
    store.select("a1");
    // Only the selected split tab shows its panes, in reading order.
    expect(shape(store)).toEqual([
      "group a",
      "tab a1 1",
      "pane left of a1",
      "pane top of a1",
      "pane bottom of a1",
      "tab a2 1",
    ]);
  });

  it("closing_current_tab_selects_the_one_after_it_else_before_it", () => {
    const before = tree([group("a", [tab("a1"), tab("a2")]), group("b", [tab("b1")])]);
    expect(nextCurrent(tree([group("a", [tab("a2")]), group("b", [tab("b1")])]), "a1", before)).toBe("a2");
    // The last tab of a group moves on to the next group.
    expect(nextCurrent(tree([group("a", [tab("a1")]), group("b", [tab("b1")])]), "a2", before)).toBe("b1");
    // Nothing after it, so the one before it.
    expect(nextCurrent(tree([group("a", [tab("a1"), tab("a2")])]), "b1", before)).toBe("a2");
  });

  it("closing_last_tab_of_last_group_selects_null", () => {
    const before = tree([group("a", [tab("a1")])]);
    expect(nextCurrent(tree([]), "a1", before)).toBeNull();
  });

  it("current_survives_a_tree_push_that_moves_it", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1"), tab("a2")]), group("b", [tab("b1")])]));
    store.select("a2");
    store.setTree(tree([group("a", [tab("a1")]), group("b", [tab("b1"), tab("a2")])]));
    expect(store.current).toBe("a2");
  });

  it("tree_push_that_drops_current_picks_a_neighbor", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1"), tab("a2"), tab("a3")])]));
    store.select("a2");
    store.setTree(tree([group("a", [tab("a1"), tab("a3")])]));
    expect(store.current).toBe("a3");
  });

  it("first_tree_selects_the_first_tab", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1")])], [tab("u1")]));
    expect(store.current).toBe("u1");
  });

  it("select_ignores_a_tab_that_is_not_in_the_tree", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1")])]));
    store.select("gone");
    expect(store.current).toBe("a1");
  });

  it("subscribers_hear_tree_and_selection_changes_until_they_leave", () => {
    const store = new Store();
    const heard = vi.fn();
    const leave = store.subscribe(heard);
    store.setTree(tree([group("a", [tab("a1"), tab("a2")])]));
    store.select("a2");
    // Selecting the tab that is already current changes nothing, so nobody is told.
    store.select("a2");
    expect(heard).toHaveBeenCalledTimes(2);
    leave();
    store.select("a1");
    expect(heard).toHaveBeenCalledTimes(2);
  });

  it("helpers_find_tabs_and_leaves", () => {
    const t = tree([group("a", [tab("a1"), tab("a2")])]);
    expect(tabAt(t, 0, 1)?.id).toBe("a2");
    expect(tabAt(t, 1, 0)).toBeUndefined();
    expect(leaves({ type: "col", sizes: [1, 1], children: [leaf("x"), leaf("y")] })).toEqual(["x", "y"]);
  });

  it("with_collapsed_changes_only_that_group", () => {
    const t = tree([group("a", [tab("a1")]), group("b", [tab("b1")])]);
    const next = withCollapsed(t, "a", true);
    expect(next.groups.map((g) => g.collapsed)).toEqual([true, false]);
    expect(t.groups[0].collapsed).toBe(false);
  });
  it("resize_rounds_to_whole_cells", () => {
    const cell = 8.4;
    const sizes = roundToCells([100, 205], 0, cell);
    // 100 px is 11.9 cells, so the first pane takes 12 and the second keeps the rest.
    expect(sizes[0]).toBeCloseTo(12 * cell);
    expect(sizes[0] + sizes[1]).toBeCloseTo(305);
    // A pane dragged to nothing still keeps one cell.
    expect(roundToCells([1, 300], 0, cell)[0]).toBeCloseTo(cell);
    const split: Split = { type: "row", children: [leaf("a"), { type: "col", children: [leaf("b"), leaf("c")], sizes: [1, 1] }], sizes: [1, 1] };
    expect(withSizesAt(split, [1], [3, 5])).toEqual({
      type: "row",
      children: [leaf("a"), { type: "col", children: [leaf("b"), leaf("c")], sizes: [3, 5] }],
      sizes: [1, 1],
    });
  });
});

describe("divider drag", () => {
  it("rounding_stays_between_the_two_panes_beside_the_divider", () => {
    const cell = 8.4;
    // The divider between panes 0 and 1 of three: pane 2 keeps its size to the last decimal.
    const sizes = roundToCells([100.3, 205.1, 333.7], 0, cell);
    expect(sizes[0]).toBeCloseTo(12 * cell);
    expect(sizes[0] + sizes[1]).toBeCloseTo(305.4);
    expect(sizes[2]).toBe(333.7);
    // The divider between panes 1 and 2: pane 0 is left alone.
    const later = roundToCells([100.3, 205.1, 333.7], 1, cell);
    expect(later[0]).toBe(100.3);
    expect(later[1]).toBeCloseTo(24 * cell);
    expect(later[1] + later[2]).toBeCloseTo(538.8);
  });

  it("rounding_keeps_each_pane_at_its_floor", () => {
    const cell = 8;
    const floor = [paneFloor(400, MIN_PANE_COLS, cell), paneFloor(400, MIN_PANE_COLS, cell)] as const;
    expect(roundToCells([75, 725], 0, cell, floor)).toEqual([80, 720]);
    expect(roundToCells([725, 75], 0, cell, floor)).toEqual([720, 80]);
  });

  it("drag_stops_at_10_columns_and_3_rows", () => {
    expect(MIN_PANE_COLS).toBe(10);
    expect(MIN_PANE_ROWS).toBe(3);
    const cols = [paneFloor(400, MIN_PANE_COLS, 8), paneFloor(400, MIN_PANE_COLS, 8)] as const;
    expect(clampPair(10, 800, cols)).toBe(80);
    expect(clampPair(795, 800, cols)).toBe(720);
    expect(clampPair(300, 800, cols)).toBe(300);
    const rows = [paneFloor(300, MIN_PANE_ROWS, 17), paneFloor(300, MIN_PANE_ROWS, 17)] as const;
    expect(clampPair(0, 600, rows)).toBe(51);
    expect(clampPair(600, 600, rows)).toBe(549);
  });

  it("a_pane_already_under_the_minimum_may_only_grow", () => {
    // A tiny window: pane 0 is 40 px, under 10 columns of 8 px.
    const floor = [paneFloor(40, MIN_PANE_COLS, 8), paneFloor(400, MIN_PANE_COLS, 8)] as const;
    expect(floor[0]).toBe(40);
    expect(clampPair(20, 440, floor)).toBe(40);
    expect(clampPair(60, 440, floor)).toBe(60);
    expect(roundToCells([40, 400], 0, 8, floor)).toEqual([40, 400]);
  });
});

describe("pane neighbors", () => {
  const row: Split = { type: "row", children: [leaf("a"), leaf("b"), leaf("c")], sizes: [1, 1, 1] };

  it("closing_a_pane_hands_typing_to_the_one_before_it", () => {
    expect(neighborOf(row, "c")).toBe("b");
    expect(neighborOf(row, "b")).toBe("a");
  });

  it("closing_the_first_pane_hands_typing_to_the_next", () => {
    expect(neighborOf(row, "a")).toBe("b");
  });

  it("neighbor_is_found_in_a_nested_split", () => {
    const nested: Split = {
      type: "row",
      children: [{ type: "col", children: [leaf("a"), leaf("b")], sizes: [1, 1] }, leaf("c")],
      sizes: [1, 1],
    };
    expect(neighborOf(nested, "b")).toBe("a");
    // Before c is the column; its pane nearest to c in order is b.
    expect(neighborOf(nested, "c")).toBe("b");
    expect(neighborOf(nested, "x")).toBeUndefined();
    expect(neighborOf(leaf("a"), "a")).toBeUndefined();
  });
});

describe("session states", () => {
  it("tab_state_is_blocked_if_any_pane_is_blocked", () => {
    const store = new Store();
    const split = tab("t", { type: "row", children: [leaf("a"), leaf("b"), leaf("c")], sizes: [1, 1, 1] });
    store.setTree(tree([], [split]));
    store.setStates([
      { session: "a", state: "working", since_ms: 1 },
      { session: "b", state: "blocked", since_ms: 2 },
      { session: "c", state: "idle", since_ms: 3 },
    ]);
    expect(store.tabState(split)).toBe("blocked");
    expect(store.stateOf("a")).toBe("working");
    expect(store.stateOf("nobody")).toBeNull();
  });

  it("tab_state_order_is_blocked_then_done_then_working_then_idle", () => {
    const store = new Store();
    const split = tab("t", { type: "row", children: [leaf("a"), leaf("b")], sizes: [1, 1] });
    store.setTree(tree([], [split]));
    const pair = (a: string | null, b: string | null) => {
      store.setStates([
        { session: "a", state: a, since_ms: 1 },
        { session: "b", state: b, since_ms: 1 },
      ]);
      return store.tabState(split);
    };
    expect(pair("idle", "blocked")).toBe("blocked");
    expect(pair("done", "blocked")).toBe("blocked");
    expect(pair("working", "done")).toBe("done");
    expect(pair("idle", "working")).toBe("working");
    expect(pair("idle", null)).toBe("idle");
    expect(pair(null, null)).toBeNull();
    // A state this client does not know yet counts as none.
    expect(pair("dreaming", null)).toBeNull();
  });

  it("collapsed_group_still_reports_blocked", () => {
    const store = new Store();
    const closed = group("g", [tab("g1"), tab("g2")], true);
    store.setTree(tree([closed, group("h", [tab("h1")])]));
    store.setStates([{ session: "s-g2", state: "blocked", since_ms: 5 }]);
    expect(store.groupBlocked(closed)).toBe(true);
    expect(store.groupBlocked(store.tree.groups[1])).toBe(false);
  });

  it("jump_goes_oldest_first_and_wraps", () => {
    const store = new Store();
    store.setTree(tree([group("g", [tab("b"), tab("c")])], [tab("a"), tab("quiet")]));
    store.setStates([
      { session: "s-a", state: "blocked", since_ms: 30 },
      { session: "s-b", state: "blocked", since_ms: 10 },
      { session: "s-c", state: "blocked", since_ms: 20 },
      { session: "s-quiet", state: "working", since_ms: 1 },
    ]);
    expect(store.nextBlocked(null)).toBe("b");
    expect(store.nextBlocked("quiet")).toBe("b");
    expect(store.nextBlocked("b")).toBe("c");
    expect(store.nextBlocked("c")).toBe("a");
    expect(store.nextBlocked("a")).toBe("b");
  });

  it("jump_with_nothing_blocked_does_nothing", () => {
    const store = new Store();
    store.setTree(tree([], [tab("a")]));
    store.setStates([{ session: "s-a", state: "done", since_ms: 1 }]);
    expect(store.nextBlocked("a")).toBeNull();
    expect(store.nextBlocked(null)).toBeNull();
  });

  it("seen_sent_when_done_tab_becomes_current", () => {
    const store = new Store();
    store.setTree(tree([], [tab("a"), tab("b")]));
    store.setStates([{ session: "s-b", state: "done", since_ms: 7 }]);
    expect(store.current).toBe("a");
    expect(store.seenToSend(true)).toEqual([]);
    store.select("b");
    expect(store.seenToSend(true)).toEqual(["s-b"]);
    // Once per done: the host's answer is on its way.
    expect(store.seenToSend(true)).toEqual([]);
    // A later done of the same session counts again.
    store.setStates([{ session: "s-b", state: "done", since_ms: 9 }]);
    expect(store.seenToSend(true)).toEqual(["s-b"]);
  });

  it("seen_not_sent_while_window_is_unfocused", () => {
    const store = new Store();
    store.setTree(tree([], [tab("a"), tab("b")]));
    store.setStates([{ session: "s-b", state: "done", since_ms: 7 }]);
    store.select("b");
    expect(store.seenToSend(false)).toEqual([]);
    expect(store.seenToSend(true)).toEqual(["s-b"]);
  });

  it("states_tell_their_own_listeners_not_the_tree_listeners", () => {
    const store = new Store();
    store.setTree(tree([], [tab("a")]));
    const tree_ = vi.fn();
    const states = vi.fn();
    store.subscribe(tree_);
    store.subscribeStates(states);
    store.setStates([{ session: "s-a", state: "working", since_ms: 1 }]);
    expect(states).toHaveBeenCalledTimes(1);
    expect(tree_).not.toHaveBeenCalled();
  });
});
