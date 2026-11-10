// The app's keys, from the final table in docs/ui-review.md. One window listener serves them all
// and leaves every key without Cmd or Ctrl at once, so plain typing costs one check. No key puts
// up a popup, toast or readout.

import type { Connection, Dir, Place, Tab } from "./connection";
import type { Panes, PaneDirection } from "./panes";
import { type Store, allTabs, findTab, withCollapsed } from "./store";

type Digit = "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";

export type Action =
  | "newTab"
  | "closePane"
  | "closeTab"
  | "reopenTab"
  | "nextTab"
  | "prevTab"
  | `group${Digit}`
  | `tab${Digit}`
  | "splitRight"
  | "splitDown"
  | "paneLeft"
  | "paneRight"
  | "paneUp"
  | "paneDown"
  | "fillTab"
  | "toggleSidebar"
  | "rename"
  | "zoomIn"
  | "zoomOut"
  | "zoomReset"
  | "jumpBlocked";

/**
 * Modifiers, then the key, joined by "+". A key with more than one binding lists them with ", ".
 * Keys are matched by place on the keyboard, so `=` with Shift is plus. Copy and paste stay with
 * the Edit menu and are not here.
 */
export const DEFAULT_KEYS: Record<Action, string> = {
  newTab: "Cmd+T",
  closePane: "Cmd+W",
  closeTab: "Cmd+Shift+W",
  reopenTab: "Cmd+Shift+T",
  nextTab: "Cmd+Shift+]",
  prevTab: "Cmd+Shift+[",
  group1: "Cmd+1",
  group2: "Cmd+2",
  group3: "Cmd+3",
  group4: "Cmd+4",
  group5: "Cmd+5",
  group6: "Cmd+6",
  group7: "Cmd+7",
  group8: "Cmd+8",
  group9: "Cmd+9",
  tab1: "Ctrl+1",
  tab2: "Ctrl+2",
  tab3: "Ctrl+3",
  tab4: "Ctrl+4",
  tab5: "Ctrl+5",
  tab6: "Ctrl+6",
  tab7: "Ctrl+7",
  tab8: "Ctrl+8",
  tab9: "Ctrl+9",
  splitRight: "Cmd+D",
  splitDown: "Cmd+Shift+D",
  // The one Option key in the defaults (pane-keys). Arrows, never a letter or digit.
  paneLeft: "Cmd+Option+Left",
  paneRight: "Cmd+Option+Right",
  paneUp: "Cmd+Option+Up",
  paneDown: "Cmd+Option+Down",
  fillTab: "Cmd+Shift+Enter",
  toggleSidebar: "Cmd+B",
  rename: "Cmd+Shift+R",
  zoomIn: "Cmd+=, Cmd+Shift+=",
  zoomOut: "Cmd+-",
  zoomReset: "Cmd+0",
  jumpBlocked: "Cmd+Shift+U",
};

export const ACTIONS = Object.keys(DEFAULT_KEYS) as Action[];

// Held down, these repeat. The rest act once per press, so a held Cmd+T opens one tab.
const REPEATS = new Set<Action>(["nextTab", "prevTab", "paneLeft", "paneRight", "paneUp", "paneDown", "zoomIn", "zoomOut"]);

const NAMED_CODES: Record<string, string> = {
  "[": "BracketLeft",
  "]": "BracketRight",
  "=": "Equal",
  "-": "Minus",
  Enter: "Enter",
  Left: "ArrowLeft",
  Right: "ArrowRight",
  Up: "ArrowUp",
  Down: "ArrowDown",
};

/** The `KeyboardEvent.code` a key name stands for. */
export function codeOf(key: string): string | undefined {
  if (/^[A-Z]$/.test(key)) return `Key${key}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  return NAMED_CODES[key];
}

function id(meta: boolean, ctrl: boolean, alt: boolean, shift: boolean, code: string): string {
  return `${meta ? "C" : ""}${ctrl ? "T" : ""}${alt ? "O" : ""}${shift ? "S" : ""} ${code}`;
}

/** One id per binding in `spec`, the same for the same keys whatever order they are written in. */
export function keyIds(spec: string): string[] {
  return spec.split(",").map((binding) => {
    const parts = binding.trim().split("+");
    // "Cmd+Shift+=" splits cleanly; the key is always the last part.
    const key = parts.pop() ?? "";
    const code = codeOf(key);
    if (code === undefined) throw new Error(`Unknown key ${key} in ${spec}`);
    const has = (name: string) => parts.includes(name);
    return id(has("Cmd"), has("Ctrl"), has("Option"), has("Shift"), code);
  });
}

const tables = new WeakMap<Record<Action, string>, Map<string, Action>>();

function tableFor(keys: Record<Action, string>): Map<string, Action> {
  let table = tables.get(keys);
  if (!table) {
    table = new Map();
    for (const action of Object.keys(keys) as Action[]) {
      for (const key of keyIds(keys[action])) table.set(key, action);
    }
    tables.set(keys, table);
  }
  return table;
}

export function matchKey(e: KeyboardEvent, keys: Record<Action, string>): Action | null {
  // Plain typing leaves here: no lookup, nothing made.
  if (!e.metaKey && !e.ctrlKey) return null;
  return tableFor(keys).get(id(e.metaKey, e.ctrlKey, e.altKey, e.shiftKey, e.code)) ?? null;
}

/** One capture listener on the window, ahead of the terminal. Returns the way to stop. */
export function installKeys(keys: Record<Action, string>, run: (action: Action) => void): () => void {
  const onKeyDown = (event: KeyboardEvent): void => {
    if (!event.metaKey && !event.ctrlKey) return;
    if (event.isComposing) return;
    const action = matchKey(event, keys);
    if (action === null) return;
    // The terminal never sees an app key, and the page does nothing of its own with it.
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat && !REPEATS.has(action)) return;
    run(action);
  };
  window.addEventListener("keydown", onKeyDown, true);
  return () => window.removeEventListener("keydown", onKeyDown, true);
}

export type ActionContext = {
  store: Store;
  panes: Pick<Panes, "focusedSession" | "focusSession" | "viewFor" | "focusNeighbor" | "toggleFill">;
  connection: Pick<Connection, "treeOp" | "list" | "create" | "kill">;
  /** Where a new shell starts when the focused pane's folder is not known. */
  home: string;
  /** What lives outside the tree and the panes. */
  ui: {
    toggleSidebar(): void;
    rename(): void;
    zoom(step: 1 | -1 | 0): void;
    note(text: string): void;
  };
};

const START_COLS = 80;
const START_ROWS = 24;
// A tab that never turns up (a failed create, a host that refused) stops being waited for.
const FOLLOW_MS = 10_000;

function selectTab(ctx: ActionContext, tab: string): void {
  if (tab === ctx.store.current) return;
  ctx.store.select(tab);
  ctx.connection.treeOp({ type: "used", tab });
}

/** Runs `then` once with the first tab `has` picks, now or in a later tree. Returns the way to stop. */
function follow(store: Store, has: (tab: Tab) => boolean, then: (tab: Tab) => void): () => void {
  let stop = (): void => {};
  const check = (): boolean => {
    const tab = allTabs(store.tree).find(has);
    if (!tab) return false;
    stop();
    then(tab);
    return true;
  };
  const unsubscribe = store.subscribe(check);
  const timer = setTimeout(() => stop(), FOLLOW_MS);
  stop = () => {
    unsubscribe();
    clearTimeout(timer);
  };
  return stop;
}

function holds(tab: Tab, session: string): boolean {
  const walk = (split: Tab["split"]): boolean =>
    split.type === "leaf" ? split.session === session : split.children.some(walk);
  return walk(tab.split);
}

/** A shell in the focused pane's live folder, as a new tab or beside the focused pane. */
async function openShell(ctx: ActionContext, dir?: Dir): Promise<void> {
  const focused = ctx.panes.focusedSession();
  if (dir && focused === undefined) return;
  const uuid = crypto.randomUUID();
  const stop = follow(
    ctx.store,
    (tab) => holds(tab, uuid),
    (tab) => {
      selectTab(ctx, tab.id);
      ctx.panes.focusSession(uuid);
    },
  );
  try {
    // Asked now, not remembered, so a `cd` a moment ago counts.
    const items = await ctx.connection.list();
    const cwd = items.find((item) => item.uuid === focused)?.cwd || ctx.home;
    const size = (focused !== undefined ? ctx.panes.viewFor(focused)?.size : undefined) ?? {
      cols: START_COLS,
      rows: START_ROWS,
    };
    const place: Place =
      dir && focused !== undefined ? { type: "split_of", session: focused, dir } : { type: "new_tab" };
    await ctx.connection.create(uuid, "shell", cwd, size.cols, size.rows, place);
  } catch (error) {
    stop();
    ctx.ui.note(`Could not open a shell: ${error}`);
  }
}

async function closePane(ctx: ActionContext): Promise<void> {
  const focused = ctx.panes.focusedSession();
  if (focused === undefined) return;
  try {
    const item = (await ctx.connection.list()).find((entry) => entry.uuid === focused);
    if (item) ctx.connection.kill(item.session);
  } catch (error) {
    ctx.ui.note(`Could not close the pane: ${error}`);
  }
}

function reopenTab(ctx: ActionContext): void {
  // The host keeps closed tabs oldest first and brings back the last one.
  const id = ctx.store.tree.closed.at(-1)?.tab.id;
  if (id === undefined) return;
  follow(ctx.store, (tab) => tab.id === id, (tab) => selectTab(ctx, tab.id));
  ctx.connection.treeOp({ type: "reopen_tab" });
}

/** The next or previous tab in sidebar order. Tabs of collapsed groups are not in that order. */
function stepTab(ctx: ActionContext, step: 1 | -1): void {
  const tabs = ctx.store.rows().flatMap((row) => (row.kind === "tab" ? [row.tab.id] : []));
  if (tabs.length === 0) return;
  const at = ctx.store.current === null ? -1 : tabs.indexOf(ctx.store.current);
  const next = at < 0 ? (step === 1 ? 0 : tabs.length - 1) : (at + step + tabs.length) % tabs.length;
  selectTab(ctx, tabs[next]);
}

/** The tab last used in group `n`, counting from 1. Past the last group nothing happens. */
function jumpToGroup(ctx: ActionContext, n: number): void {
  const group = ctx.store.tree.groups[n - 1];
  if (!group || group.tabs.length === 0) return;
  const best = group.tabs.reduce((a, b) => (b.last_used > a.last_used ? b : a));
  selectTab(ctx, best.id);
}

/** Tab `n` of the list the current tab is in: its group, or the ungrouped tabs. */
function jumpToTab(ctx: ActionContext, n: number): void {
  const { tree, current } = ctx.store;
  if (current === null || !findTab(tree, current)) return;
  const list =
    tree.groups.find((group) => group.tabs.some((tab) => tab.id === current))?.tabs ?? tree.ungrouped;
  const tab = list[n - 1];
  if (tab) selectTab(ctx, tab.id);
}

/** The next blocked tab, oldest first. A collapsed group it is in opens first. */
function jumpBlocked(ctx: ActionContext): void {
  const id = ctx.store.nextBlocked(ctx.store.current);
  if (id === null) return;
  const group = ctx.store.tree.groups.find((g) => g.tabs.some((tab) => tab.id === id));
  if (group?.collapsed) {
    // Shown at once, as a click on the folder does, then the host's pushed tree says the same.
    ctx.store.setTree(withCollapsed(ctx.store.tree, group.key, false));
    ctx.connection.treeOp({ type: "collapse_group", group: group.key, collapsed: false });
  }
  selectTab(ctx, id);
}

const PANE_DIRECTIONS: Partial<Record<Action, PaneDirection>> = {
  paneLeft: "left",
  paneRight: "right",
  paneUp: "up",
  paneDown: "down",
};

export function runAction(a: Action, ctx: ActionContext): void {
  if (a.startsWith("group")) return jumpToGroup(ctx, Number(a.slice(5)));
  if (a.startsWith("tab")) return jumpToTab(ctx, Number(a.slice(3)));
  const direction = PANE_DIRECTIONS[a];
  if (direction) return ctx.panes.focusNeighbor(direction);
  switch (a) {
    case "newTab":
      void openShell(ctx);
      return;
    case "splitRight":
      void openShell(ctx, "right");
      return;
    case "splitDown":
      void openShell(ctx, "down");
      return;
    case "closePane":
      void closePane(ctx);
      return;
    case "closeTab":
      if (ctx.store.current !== null) ctx.connection.treeOp({ type: "close_tab", tab: ctx.store.current });
      return;
    case "reopenTab":
      reopenTab(ctx);
      return;
    case "nextTab":
      stepTab(ctx, 1);
      return;
    case "prevTab":
      stepTab(ctx, -1);
      return;
    case "fillTab":
      ctx.panes.toggleFill();
      return;
    case "toggleSidebar":
      ctx.ui.toggleSidebar();
      return;
    case "rename":
      ctx.ui.rename();
      return;
    case "zoomIn":
      ctx.ui.zoom(1);
      return;
    case "zoomOut":
      ctx.ui.zoom(-1);
      return;
    case "zoomReset":
      ctx.ui.zoom(0);
      return;
    case "jumpBlocked":
      jumpBlocked(ctx);
      return;
  }
}
