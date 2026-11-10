import { describe, expect, it, vi } from "vitest";

import type { Group, Split, Tab, Tree } from "./connection";
import { ACTIONS, type ActionContext, DEFAULT_KEYS, codeOf, keyIds, matchKey, runAction } from "./keys";
import { Store } from "./store";

function leaf(session: string): Split {
  return { type: "leaf", session };
}

function tab(id: string, lastUsed = 0): Tab {
  return { id, name: null, pinned: false, split: leaf(`s-${id}`), last_used: lastUsed };
}

function group(key: string, tabs: Tab[], collapsed = false): Group {
  return { key, path: `/code/${key}`, name: null, branch: "main", collapsed, tabs };
}

function tree(groups: Group[], ungrouped: Tab[] = []): Tree {
  return { groups, ungrouped, closed: [], defaults: {} };
}

function context(store: Store): ActionContext & { treeOp: ReturnType<typeof vi.fn> } {
  const treeOp = vi.fn();
  return {
    store,
    treeOp,
    panes: {
      focusedSession: () => undefined,
      focusSession: vi.fn(),
      viewFor: () => undefined,
      focusNeighbor: vi.fn(),
      toggleFill: vi.fn(),
    },
    connection: { treeOp, list: vi.fn(async () => []), create: vi.fn(async () => 1), kill: vi.fn() },
    home: "/home",
    ui: { toggleSidebar: vi.fn(), rename: vi.fn(), zoom: vi.fn(), note: vi.fn() },
  };
}

/** The key event a binding such as "Cmd+Shift+T" stands for. */
function press(binding: string): KeyboardEvent {
  const parts = binding.split("+");
  const key = parts.pop() ?? "";
  return new KeyboardEvent("keydown", {
    code: codeOf(key),
    metaKey: parts.includes("Cmd"),
    ctrlKey: parts.includes("Ctrl"),
    altKey: parts.includes("Option"),
    shiftKey: parts.includes("Shift"),
  });
}

describe("keys", () => {
  it("every_action_has_a_default", () => {
    expect(ACTIONS.length).toBeGreaterThan(0);
    for (const action of ACTIONS) {
      expect(DEFAULT_KEYS[action]).toBeTruthy();
      for (const binding of DEFAULT_KEYS[action].split(", ")) {
        expect(matchKey(press(binding), DEFAULT_KEYS)).toBe(action);
      }
    }
  });

  it("no_default_uses_option_with_a_letter_or_digit", () => {
    for (const action of ACTIONS) {
      for (const binding of DEFAULT_KEYS[action].split(", ")) {
        const parts = binding.split("+");
        const key = parts.pop() ?? "";
        if (parts.includes("Option")) expect(key).not.toMatch(/^[A-Z0-9]$/);
      }
    }
  });

  it("no_two_actions_share_a_key", () => {
    const all = ACTIONS.flatMap((action) => keyIds(DEFAULT_KEYS[action]));
    expect(new Set(all).size).toBe(all.length);
  });

  it("plain_typing_and_copy_paste_match_nothing", () => {
    expect(matchKey(new KeyboardEvent("keydown", { code: "KeyT" }), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(new KeyboardEvent("keydown", { code: "KeyT", shiftKey: true }), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(press("Cmd+C"), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(press("Cmd+V"), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(press("Ctrl+C"), DEFAULT_KEYS)).toBeNull();
  });

  it("cmd_3_jumps_to_last_used_tab_of_third_group", () => {
    const store = new Store();
    store.setTree(
      tree([group("a", [tab("a1", 1)]), group("b", [tab("b1", 2)]), group("c", [tab("c1", 5), tab("c2", 9), tab("c3", 2)])]),
    );
    const ctx = context(store);
    runAction(matchKey(press("Cmd+3"), DEFAULT_KEYS) ?? "newTab", ctx);
    expect(store.current).toBe("c2");
    expect(ctx.treeOp).toHaveBeenCalledWith({ type: "used", tab: "c2" });
  });

  it("ctrl_2_picks_second_tab_in_current_group", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1"), tab("a2")]), group("b", [tab("b1"), tab("b2"), tab("b3")])]));
    store.select("b3");
    runAction(matchKey(press("Ctrl+2"), DEFAULT_KEYS) ?? "newTab", context(store));
    expect(store.current).toBe("b2");
  });

  it("next_tab_skips_collapsed_group", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1")]), group("b", [tab("b1")], true), group("c", [tab("c1")])]));
    store.select("a1");
    const ctx = context(store);
    runAction("nextTab", ctx);
    expect(store.current).toBe("c1");
    runAction("prevTab", ctx);
    expect(store.current).toBe("a1");
  });

  it("jump_past_the_last_group_does_nothing", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1")]), group("b", [tab("b1")])]));
    store.select("b1");
    const ctx = context(store);
    runAction("group5", ctx);
    expect(store.current).toBe("b1");
    expect(ctx.treeOp).not.toHaveBeenCalled();
  });
});
