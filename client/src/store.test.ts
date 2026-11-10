import { describe, expect, it, vi } from "vitest";

import type { Group, Split, Tab, Tree } from "./connection";
import { Store, leaves, nextCurrent, tabAt, withCollapsed } from "./store";

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
});
