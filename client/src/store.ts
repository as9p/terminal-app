// The tree as the window sees it, and which tab is on screen. Knows nothing about the DOM.

import type { Group, Split, Tab, Tree } from "./connection";

export type Row =
  | { kind: "group"; group: Group }
  | { kind: "tab"; tab: Tab; depth: 0 | 1 }
  | { kind: "pane"; session: string; tab: string };

const EMPTY: Tree = { groups: [], ungrouped: [], closed: [], defaults: {} };

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

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(): void {
    for (const fn of this.listeners) fn();
  }
}
