import { describe, expect, it, vi } from "vitest";

import type { Group, Split, Tab, Tree } from "./connection";
import { ACTIONS, type ActionContext, DEFAULT_KEYS, installKeys, keyIds, matchKey, runAction } from "./keys";
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
      leaveFill: vi.fn(),
    },
    connection: { treeOp, list: vi.fn(async () => []), create: vi.fn(async () => 1), kill: vi.fn() },
    home: "/home",
    ui: { toggleSidebar: vi.fn(), rename: vi.fn(), zoom: vi.fn(), note: vi.fn() },
  };
}

// A US keyboard: what each key name types, without and with Shift, and where it sits.
const US_SHIFTED: Record<string, string> = { "[": "{", "]": "}", "=": "+", "-": "_" };
const US_CODES: Record<string, string> = { "[": "BracketLeft", "]": "BracketRight", "=": "Equal", "-": "Minus" };
const US_DIGITS_SHIFTED = ")!@#$%^&*(";

function us(name: string, shift: boolean): { key: string; code: string } {
  if (/^[A-Z]$/.test(name)) return { key: shift ? name : name.toLowerCase(), code: `Key${name}` };
  if (/^[0-9]$/.test(name)) return { key: shift ? US_DIGITS_SHIFTED[Number(name)] : name, code: `Digit${name}` };
  if (name === "Enter") return { key: "Enter", code: "Enter" };
  if (["Left", "Right", "Up", "Down"].includes(name)) return { key: `Arrow${name}`, code: `Arrow${name}` };
  return { key: shift ? US_SHIFTED[name] : name, code: US_CODES[name] };
}

type Mods = { meta?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean };

function keyEvent(key: string, code: string, mods: Mods = {}, repeat = false): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key,
    code,
    metaKey: mods.meta ?? false,
    ctrlKey: mods.ctrl ?? false,
    altKey: mods.alt ?? false,
    shiftKey: mods.shift ?? false,
    repeat,
    bubbles: true,
    cancelable: true,
  });
}

/** The key event a binding such as "Cmd+Shift+T" stands for, typed on a US keyboard. */
function press(binding: string, repeat = false): KeyboardEvent {
  const parts = binding.split("+");
  const name = parts.pop() ?? "";
  const shift = parts.includes("Shift");
  const { key, code } = us(name, shift);
  const mods = { meta: parts.includes("Cmd"), ctrl: parts.includes("Ctrl"), alt: parts.includes("Option"), shift };
  return keyEvent(key, code, mods, repeat);
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

  it("next_tab_from_a_tab_in_a_collapsed_group_steps_from_that_group", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1"), tab("a2")]), group("b", [tab("b1")], true), group("c", [tab("c1"), tab("c2")])]));
    store.select("b1");
    const ctx = context(store);
    runAction("nextTab", ctx);
    expect(store.current).toBe("c1");
    store.select("b1");
    runAction("prevTab", ctx);
    expect(store.current).toBe("a2");
  });

  it("stepping_from_a_collapsed_group_wraps", () => {
    const store = new Store();
    store.setTree(tree([group("a", [tab("a1")], true), group("b", [tab("b1"), tab("b2")])]));
    store.select("a1");
    const ctx = context(store);
    runAction("prevTab", ctx);
    expect(store.current).toBe("b2");
    store.setTree(tree([group("b", [tab("b1"), tab("b2")]), group("a", [tab("a1")], true)]));
    store.select("a1");
    runAction("nextTab", ctx);
    expect(store.current).toBe("b1");
  });

  it("split_leaves_fill_when_the_new_pane_arrives_and_before_focusing_it", async () => {
    const store = new Store();
    store.setTree(tree([], [tab("a")]));
    store.select("a");
    const ctx = context(store);
    ctx.panes.focusedSession = () => "s-a";
    const order: string[] = [];
    vi.mocked(ctx.panes.leaveFill).mockImplementation(() => order.push("leaveFill"));
    vi.mocked(ctx.panes.focusSession).mockImplementation((session: string) => order.push(`focus ${session}`));
    const uuid = vi.spyOn(crypto, "randomUUID").mockReturnValue("0-0-0-0-new");
    runAction("splitRight", ctx);
    await vi.waitFor(() => expect(ctx.connection.create).toHaveBeenCalled());
    // Still filled until the split is real.
    expect(ctx.panes.leaveFill).not.toHaveBeenCalled();
    const split: Split = { type: "row", children: [leaf("s-a"), leaf("0-0-0-0-new")], sizes: [1, 1] };
    store.setTree(tree([], [{ ...tab("a"), split }]));
    expect(order).toEqual(["leaveFill", "focus 0-0-0-0-new"]);
    uuid.mockRestore();
  });

  it("split_that_fails_to_open_keeps_fill", async () => {
    const store = new Store();
    store.setTree(tree([], [tab("a")]));
    const ctx = context(store);
    ctx.panes.focusedSession = () => "s-a";
    ctx.connection.create = vi.fn(async () => {
      throw new Error("no host");
    });
    runAction("splitRight", ctx);
    runAction("splitDown", ctx);
    await vi.waitFor(() => expect(ctx.ui.note).toHaveBeenCalledTimes(2));
    expect(ctx.panes.leaveFill).not.toHaveBeenCalled();
  });

  it("new_tab_does_not_leave_fill", async () => {
    const store = new Store();
    store.setTree(tree([], [tab("a")]));
    const ctx = context(store);
    ctx.panes.focusedSession = () => "s-a";
    const uuid = vi.spyOn(crypto, "randomUUID").mockReturnValue("0-0-0-0-tab");
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(ctx.connection.create).toHaveBeenCalled());
    store.setTree(tree([], [tab("a"), { ...tab("n"), split: leaf("0-0-0-0-tab") }]));
    expect(ctx.panes.focusSession).toHaveBeenCalledWith("0-0-0-0-tab");
    expect(ctx.panes.leaveFill).not.toHaveBeenCalled();
    uuid.mockRestore();
  });
});

describe("keyboard layouts", () => {
  it("shift_does_not_break_letters", () => {
    expect(matchKey(keyEvent("t", "KeyT", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("reopenTab");
    expect(matchKey(keyEvent("T", "KeyT", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("reopenTab");
    expect(matchKey(keyEvent("T", "KeyT", { meta: true }), DEFAULT_KEYS)).toBe("newTab");
  });

  it("us_brackets_and_plus_match_with_shift_as_typed", () => {
    expect(matchKey(keyEvent("}", "BracketRight", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("nextTab");
    expect(matchKey(keyEvent("]", "BracketRight", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("nextTab");
    expect(matchKey(keyEvent("{", "BracketLeft", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("prevTab");
    expect(matchKey(keyEvent("+", "Equal", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("zoomIn");
    expect(matchKey(keyEvent("=", "Equal", { meta: true }), DEFAULT_KEYS)).toBe("zoomIn");
    expect(matchKey(keyEvent("]", "BracketRight", { meta: true }), DEFAULT_KEYS)).toBeNull();
  });

  it("non_latin_layouts_fall_back_to_the_key_place", () => {
    // Russian: Cmd+T types "е" (Cyrillic), the key is still KeyT.
    expect(matchKey(keyEvent("е", "KeyT", { meta: true }), DEFAULT_KEYS)).toBe("newTab");
    expect(matchKey(keyEvent("Е", "KeyT", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("reopenTab");
    // Russian has "ъ" where US has "]".
    expect(matchKey(keyEvent("Ъ", "BracketRight", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("nextTab");
    // Greek: Cmd+W types "ς".
    expect(matchKey(keyEvent("ς", "KeyW", { meta: true }), DEFAULT_KEYS)).toBe("closePane");
  });

  it("non_latin_plain_keys_still_leave_at_once", () => {
    expect(matchKey(keyEvent("е", "KeyT"), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(keyEvent("Е", "KeyT", { shift: true }), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(keyEvent("е", "KeyT", { alt: true }), DEFAULT_KEYS)).toBeNull();
  });

  it("dvorak_letters_follow_the_label_not_the_place", () => {
    // On Dvorak the key labeled W sits where US has the comma, and US W types a comma.
    expect(matchKey(keyEvent("w", "Comma", { meta: true }), DEFAULT_KEYS)).toBe("closePane");
    expect(matchKey(keyEvent(",", "KeyW", { meta: true }), DEFAULT_KEYS)).toBeNull();
    expect(matchKey(keyEvent("t", "KeyK", { meta: true }), DEFAULT_KEYS)).toBe("newTab");
    expect(matchKey(keyEvent("d", "KeyH", { meta: true }), DEFAULT_KEYS)).toBe("splitRight");
  });

  it("german_plus_and_minus_zoom", () => {
    expect(matchKey(keyEvent("+", "BracketRight", { meta: true }), DEFAULT_KEYS)).toBe("zoomIn");
    expect(matchKey(keyEvent("-", "Slash", { meta: true }), DEFAULT_KEYS)).toBe("zoomOut");
    expect(matchKey(keyEvent("=", "Digit0", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("zoomIn");
  });

  it("french_digits_fall_back_to_the_place", () => {
    expect(matchKey(keyEvent("&", "Digit1", { meta: true }), DEFAULT_KEYS)).toBe("group1");
    expect(matchKey(keyEvent("é", "Digit2", { ctrl: true }), DEFAULT_KEYS)).toBe("tab2");
    expect(matchKey(keyEvent("ç", "Digit9", { meta: true }), DEFAULT_KEYS)).toBe("group9");
    expect(matchKey(keyEvent("à", "Digit0", { meta: true }), DEFAULT_KEYS)).toBe("zoomReset");
  });

  it("dead_or_unidentified_keys_fall_back_to_the_place", () => {
    expect(matchKey(keyEvent("Dead", "KeyT", { meta: true }), DEFAULT_KEYS)).toBe("newTab");
    expect(matchKey(keyEvent("Unidentified", "KeyB", { meta: true }), DEFAULT_KEYS)).toBe("toggleSidebar");
    expect(matchKey(keyEvent("Dead", "Equal", { meta: true }), DEFAULT_KEYS)).toBe("zoomIn");
  });

  it("option_arrows_and_enter_match", () => {
    expect(matchKey(keyEvent("ArrowLeft", "ArrowLeft", { meta: true, alt: true }), DEFAULT_KEYS)).toBe("paneLeft");
    expect(matchKey(keyEvent("ArrowDown", "ArrowDown", { meta: true, alt: true }), DEFAULT_KEYS)).toBe("paneDown");
    expect(matchKey(keyEvent("Enter", "Enter", { meta: true, shift: true }), DEFAULT_KEYS)).toBe("fillTab");
  });
});

describe("installKeys", () => {
  function setup() {
    const run = vi.fn();
    const terminal = document.createElement("textarea");
    document.body.append(terminal);
    const seen = vi.fn();
    terminal.addEventListener("keydown", seen);
    const stop = installKeys(DEFAULT_KEYS, run);
    const send = (event: KeyboardEvent) => {
      terminal.dispatchEvent(event);
      return event;
    };
    const done = () => {
      stop();
      terminal.remove();
    };
    return { run, seen, send, done };
  }

  it("cmd_shortcut_is_stopped_before_the_terminal", () => {
    const { run, seen, send, done } = setup();
    const event = send(press("Cmd+T"));
    expect(run).toHaveBeenCalledWith("newTab");
    expect(seen).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(true);
    done();
  });

  it("plain_key_is_never_stopped_or_prevented", () => {
    const { run, seen, send, done } = setup();
    for (const event of [keyEvent("t", "KeyT"), keyEvent("T", "KeyT", { shift: true }), keyEvent("Escape", "Escape")]) {
      send(event);
      expect(event.defaultPrevented).toBe(false);
    }
    // Cmd+C is not an app key, so the terminal and the Edit menu get it.
    const copy = send(press("Cmd+C"));
    expect(copy.defaultPrevented).toBe(false);
    expect(seen).toHaveBeenCalledTimes(4);
    expect(run).not.toHaveBeenCalled();
    done();
  });

  it("repeat_is_filtered_only_for_actions_that_ignore_it", () => {
    const { run, seen, send, done } = setup();
    const held = send(press("Cmd+T", true));
    expect(run).not.toHaveBeenCalled();
    // Still the app's key: a held Cmd+T never types into the terminal.
    expect(held.defaultPrevented).toBe(true);
    send(press("Cmd+=", true));
    send(press("Cmd+Shift+]", true));
    send(press("Cmd+Option+Left", true));
    expect(run.mock.calls.map(([action]) => action)).toEqual(["zoomIn", "nextTab", "paneLeft"]);
    expect(seen).not.toHaveBeenCalled();
    done();
  });
});

describe("jump to blocked", () => {
  it("jump_key_is_cmd_shift_u", () => {
    expect(matchKey(press("Cmd+Shift+U"), DEFAULT_KEYS)).toBe("jumpBlocked");
  });

  it("jump_expands_a_collapsed_group", () => {
    const store = new Store();
    store.setTree(tree([group("g", [tab("g1")], true)], [tab("a")]));
    store.setStates([{ session: "s-g1", state: "blocked", since_ms: 1 }]);
    const ctx = context(store);
    runAction("jumpBlocked", ctx);
    expect(store.current).toBe("g1");
    expect(store.tree.groups[0].collapsed).toBe(false);
    expect(ctx.treeOp).toHaveBeenCalledWith({ type: "collapse_group", group: "g", collapsed: false });
    expect(ctx.treeOp).toHaveBeenCalledWith({ type: "used", tab: "g1" });
  });

  it("jump_with_nothing_blocked_leaves_the_tab_alone", () => {
    const store = new Store();
    store.setTree(tree([], [tab("a"), tab("b")]));
    store.select("b");
    const ctx = context(store);
    runAction("jumpBlocked", ctx);
    expect(store.current).toBe("b");
    expect(ctx.treeOp).not.toHaveBeenCalled();
  });
});
