// The tree as the window sees it, and which tab is on screen. Knows nothing about the DOM.

import type { AgentState, Group, Split, StateItem, Tab, Tree } from "./connection";

export type Row =
  | { kind: "group"; group: Group }
  | { kind: "tab"; tab: Tab; depth: 0 | 1 }
  | { kind: "pane"; session: string; tab: string };

const EMPTY: Tree = { groups: [], ungrouped: [], closed: [], defaults: {} };

// A tab shows the loudest state of its panes, in this order.
const LOUDNESS: readonly AgentState[] = ["blocked", "done", "working", "idle"];

/** Session uuids left to right, top to bottom. */
export function leaves(split: Split): string[] {
  if (split.type === "leaf") return [split.session];
  return split.children.flatMap(leaves);
}

export function tabAt(tree: Tree, groupIndex: number, tabIndex: number): Tab | undefined {
  return tree.groups[groupIndex]?.tabs[tabIndex];
}

/** Every tab in sidebar order: ungrouped first, then each group's tabs. */
export function allTabs(tree: Tree): Tab[] {
  return [...tree.ungrouped, ...tree.groups.flatMap((group) => group.tabs)];
}

export function findTab(tree: Tree, id: string): Tab | undefined {
  return allTabs(tree).find((tab) => tab.id === id);
}

/**
 * The tab to show once `closed` is gone from `tree`: the first tab after it in `before` that is
 * still there, else the nearest one before it, else the first tab left, else null.
 */
export function nextCurrent(tree: Tree, closed: string, before: Tree): string | null {
  const left = new Set(allTabs(tree).map((tab) => tab.id));
  const order = allTabs(before).map((tab) => tab.id);
  const at = order.indexOf(closed);
  if (at >= 0) {
    for (let i = at + 1; i < order.length; i++) if (left.has(order[i])) return order[i];
    for (let i = at - 1; i >= 0; i--) if (left.has(order[i])) return order[i];
  }
  return allTabs(tree)[0]?.id ?? null;
}

/**
 * Pane sizes in pixels along a split, after a divider drag, made whole numbers of `cell` so no
 * pane ends in a sliver of a column or row. Each pane but the last is rounded and keeps at least
 * one cell; the last takes what is left, so the total stays the same.
 */
export function roundToCells(px: number[], cell: number): number[] {
  if (px.length === 0 || !(cell > 0)) return px.slice();
  const total = px.reduce((sum, size) => sum + size, 0);
  const sizes = px.slice(0, -1).map((size) => Math.max(1, Math.round(size / cell)) * cell);
  const used = sizes.reduce((sum, size) => sum + size, 0);
  return [...sizes, Math.max(cell, total - used)];
}

/** A copy of `split` with new sizes on the box at `path` (child indexes from the top). */
export function withSizesAt(split: Split, path: number[], sizes: number[]): Split {
  if (split.type === "leaf") return split;
  if (path.length === 0) return { ...split, sizes: sizes.slice() };
  const [index, ...rest] = path;
  return {
    ...split,
    children: split.children.map((child, i) => (i === index ? withSizesAt(child, rest, sizes) : child)),
  };
}

/** A copy of `tree` with one group opened or closed, for showing a click before the host answers. */
export function withCollapsed(tree: Tree, key: string, collapsed: boolean): Tree {
  return {
    ...tree,
    groups: tree.groups.map((group) => (group.key === key ? { ...group, collapsed } : group)),
  };
}

export class Store {
  tree: Tree = EMPTY;
  current: string | null = null;
  private readonly listeners = new Set<() => void>();
  private states = new Map<string, { state: AgentState; since: number }>();
  private readonly stateListeners = new Set<() => void>();
  // Sessions a `seen` went out for, with the `since_ms` of the done it was for.
  private readonly seenSent = new Map<string, number>();

  /** Takes a pushed tree. The current tab stays wherever it moved to, or a neighbor takes over. */
  setTree(tree: Tree): void {
    const before = this.tree;
    this.tree = tree;
    if (this.current === null || !findTab(tree, this.current)) {
      this.current =
        this.current === null ? (allTabs(tree)[0]?.id ?? null) : nextCurrent(tree, this.current, before);
    }
    this.notify();
  }

  select(tab: string): void {
    if (tab === this.current || !findTab(this.tree, tab)) return;
    this.current = tab;
    this.notify();
  }

  currentTab(): Tab | undefined {
    return this.current === null ? undefined : findTab(this.tree, this.current);
  }

  /** The sidebar's lines. Panes are listed only under the current tab, and only for a split. */
  rows(): Row[] {
    const rows: Row[] = [];
    const addTab = (tab: Tab, depth: 0 | 1) => {
      rows.push({ kind: "tab", tab, depth });
      if (tab.id !== this.current || tab.split.type === "leaf") return;
      for (const session of leaves(tab.split)) rows.push({ kind: "pane", session, tab: tab.id });
    };
    for (const tab of this.tree.ungrouped) addTab(tab, 0);
    for (const group of this.tree.groups) {
      rows.push({ kind: "group", group });
      if (!group.collapsed) for (const tab of group.tabs) addTab(tab, 1);
    }
    return rows;
  }

  /** Takes the host's states. Tells only the state listeners, so no row or pane is built again. */
  setStates(items: StateItem[]): void {
    const states = new Map<string, { state: AgentState; since: number }>();
    for (const item of items) {
      const state = LOUDNESS.find((known) => known === item.state);
      if (state) states.set(item.session, { state, since: item.since_ms });
    }
    this.states = states;
    for (const session of this.seenSent.keys()) {
      if (states.get(session)?.state !== "done") this.seenSent.delete(session);
    }
    for (const fn of this.stateListeners) fn();
  }

  /** A session's state. Null for a session with no hook event, or one the host did not name. */
  stateOf(session: string): AgentState | null {
    return this.states.get(session)?.state ?? null;
  }

  /** The loudest state of a tab's panes: blocked, then done, then working, then idle. */
  tabState(tab: Tab): AgentState | null {
    let best = LOUDNESS.length;
    for (const session of leaves(tab.split)) {
      const state = this.stateOf(session);
      if (state) best = Math.min(best, LOUDNESS.indexOf(state));
    }
    return LOUDNESS[best] ?? null;
  }

  /** True when any session in the group is blocked, collapsed or not. */
  groupBlocked(group: Group): boolean {
    return group.tabs.some((tab) => this.tabState(tab) === "blocked");
  }

  /**
   * The blocked tab after `after`, oldest blocked first, wrapping round. A tab that is not
   * blocked, or null, gives the oldest. Null when nothing is blocked.
   */
  nextBlocked(after: string | null): string | null {
    const blocked: { id: string; since: number }[] = [];
    for (const tab of allTabs(this.tree)) {
      let since = Infinity;
      for (const session of leaves(tab.split)) {
        const entry = this.states.get(session);
        if (entry?.state === "blocked") since = Math.min(since, entry.since);
      }
      if (since !== Infinity) blocked.push({ id: tab.id, since });
    }
    // Sorting keeps sidebar order between tabs blocked at the same moment.
    blocked.sort((a, b) => a.since - b.since);
    if (blocked.length === 0) return null;
    const at = blocked.findIndex((entry) => entry.id === after);
    return blocked[(at + 1) % blocked.length].id;
  }

  /**
   * The done sessions of the current tab to send `seen` for, each once per done. Nothing while
   * the window does not have focus, since nobody is looking.
   */
  seenToSend(windowFocused: boolean): string[] {
    const tab = this.currentTab();
    if (!windowFocused || !tab) return [];
    const out: string[] = [];
    for (const session of leaves(tab.split)) {
      const entry = this.states.get(session);
      if (entry?.state !== "done" || this.seenSent.get(session) === entry.since) continue;
      this.seenSent.set(session, entry.since);
      out.push(session);
    }
    return out;
  }

  subscribeStates(fn: () => void): () => void {
    this.stateListeners.add(fn);
    return () => this.stateListeners.delete(fn);
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(): void {
    for (const fn of this.listeners) fn();
  }
}
