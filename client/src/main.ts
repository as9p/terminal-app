import "@xterm/xterm/css/xterm.css";
import "./fonts.css";
import "./styles.css";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import {
  Connection,
  ConnectionFailure,
  HostError,
  type HostAddress,
  type Op,
  type Tree,
} from "./connection";
import { installFocusRule, refocus } from "./focus";
import { DEFAULT_KEYS, type Action, installKeys, runAction } from "./keys";
import { type MarkerChange, resyncTicks, setMarkerStates } from "./marker";
import { Panes } from "./panes";
import { Sessions } from "./sessions";
import { loadSettings, saveSettings } from "./settings";
import { Sidebar, clampWidth } from "./sidebar";
import { Store, allTabs, leaves, withCollapsed } from "./store";
import { type Renderer, TerminalView, terminalFontReady } from "./terminal";
import { theme } from "./theme";
import { applyZoom, clampSize, nextSize } from "./zoom";

type HostConnection = HostAddress & { home: string };

const RETRY_MS = 1000;
const RETRY_MAX_MS = 30_000;
const NOTE_MS = 4000;
const LOST_TEXT = "Lost the host, reconnecting";

/** Plain text over the top of the terminal area. A note lasts a moment, then whatever was there returns. */
class StatusBar {
  private readonly element: HTMLElement;
  private lasting = "";
  private noteTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(element: HTMLElement) {
    this.element = element;
  }

  show(text: string): void {
    this.lasting = text;
    if (this.noteTimer === undefined) this.render(text);
  }

  clear(): void {
    this.show("");
  }

  note(text: string): void {
    clearTimeout(this.noteTimer);
    this.render(text);
    this.noteTimer = setTimeout(() => {
      this.noteTimer = undefined;
      this.render(this.lasting);
    }, NOTE_MS);
  }

  private render(text: string): void {
    this.element.textContent = text;
    this.element.hidden = text === "";
  }
}

function inTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

function findHost(): Promise<HostConnection> {
  if (import.meta.env.DEV && !inTauri()) {
    const query = new URLSearchParams(location.search);
    return Promise.resolve({
      port: Number(query.get("port")),
      token: query.get("token") ?? "",
      home: query.get("home") ?? "",
    });
  }
  return invoke<HostConnection>("host_connection");
}

/** TA_RENDERER from a speed run, or `renderer` in the dev page's address. WebGL otherwise. */
async function findRenderer(): Promise<Renderer> {
  let value: string | null;
  if (import.meta.env.DEV && !inTauri()) {
    value = new URLSearchParams(location.search).get("renderer");
  } else {
    try {
      value = await invoke<string | null>("renderer");
    } catch {
      value = null;
    }
  }
  return value === "dom" ? "dom" : "webgl";
}

// A native write, because WKWebView can refuse navigator.clipboard outside a user gesture.
function copy(text: string): Promise<void> {
  if (import.meta.env.DEV && !inTauri()) return navigator.clipboard.writeText(text);
  return writeText(text);
}

function describe(error: unknown, port: number | undefined): string {
  if (error instanceof ConnectionFailure) {
    switch (error.kind) {
      case "proto_mismatch":
        return "The running host is a different version. Nothing was changed. Quit the host to update it.";
      case "token_refused":
        return `The host on port ${port} refused this app's token.`;
      case "not_our_host":
        return `Something answers on port ${port}, but it is not this app's host.`;
      case "lost":
        return LOST_TEXT;
    }
  }
  if (error instanceof HostError) return `The host says: ${error.message}`;
  return String(error);
}

const LAST_TAB_KEY = "lastTab";
// Only a size to start the shell at: the view sends its real size when it attaches.
const START_COLS = 80;
const START_ROWS = 24;

/** The id of the tab the window last showed. Lost if storage is turned off. */
function lastTab(): string | undefined {
  try {
    return localStorage.getItem(LAST_TAB_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

function rememberTab(id: string): void {
  try {
    localStorage.setItem(LAST_TAB_KEY, id);
  } catch {
    // Full or turned off: the next window starts on the first tab instead.
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function folderName(path: string): string {
  const parts = path.split("/").filter((part) => part !== "");
  return parts[parts.length - 1] ?? path;
}

function sessionsIn(tree: Tree): Set<string> {
  return new Set(allTabs(tree).flatMap((tab) => leaves(tab.split)));
}

/**
 * Someone moving through the sidebar's rows with the keyboard, typing a name, or in a context menu
 * opened from the keyboard (it lives in the body, not the sidebar) keeps their place.
 */
export function focusHeldAside(sidebar: HTMLElement): boolean {
  const active = document.activeElement;
  return sidebar.contains(active) || active?.closest('[role="menu"]') != null;
}

async function main(): Promise<void> {
  const statusElement = document.querySelector<HTMLElement>("#status");
  const parent = document.querySelector<HTMLElement>("#terminal");
  const sidebarElement = document.querySelector<HTMLElement>("#sidebar");
  if (!statusElement || !parent || !sidebarElement) return;
  statusElement.style.background = theme.black ?? "";
  statusElement.style.color = theme.foreground ?? "";
  const bar = new StatusBar(statusElement);

  const [renderer, settings] = await Promise.all([
    findRenderer(),
    loadSettings(),
    terminalFontReady(),
  ]);

  let connection: Connection | undefined;
  let home = "";
  let fontPx = clampSize(settings.fontSize);
  let bringingUp = false;
  // True once this connection's first tree is in, so an empty tree after that means the last
  // tab closed rather than a fresh host.
  let settled = false;
  // Session numbers start over in a new host, so a pane is known by its uuid and numbered here,
  // in a new Sessions for each connection.
  let sessions = new Sessions();
  const titles = new Map<string, string>();
  const folders = new Map<string, string>();

  const store = new Store();

  async function refreshSessions(next: Connection, known: Sessions): Promise<void> {
    for (const item of await next.list()) {
      known.learn(item);
      if (item.title && !titles.has(item.uuid)) titles.set(item.uuid, item.title);
      if (item.cwd) folders.set(item.uuid, folderName(item.cwd));
    }
  }

  async function attachView(view: TerminalView, uuid: string): Promise<void> {
    const next = connection;
    if (!next) return;
    const known = sessions;
    try {
      let session = known.number(uuid);
      if (session === undefined) {
        // A pane this client created gets its number from `created`; only others need a `list`.
        session = await known.numberOf(uuid, () => refreshSessions(next, known));
        // Its pane may have closed while it waited.
        if (panes.viewFor(uuid) !== view) return;
      }
      // Gone already: the tree push that drops its pane is on its way.
      if (session === undefined || connection !== next) return;
      await view.attach(next, session);
    } catch (error) {
      if (connection === next) bar.note(describe(error, undefined));
    }
  }

  function makeView(container: HTMLElement, uuid: string): TerminalView {
    const view = new TerminalView(
      container,
      {
        onSelection: (text) => {
          copy(text).catch((error: unknown) => bar.note(`Could not copy the selection: ${error}`));
        },
        // A clean exit takes the pane out of the tree, and the pushed tree ends the view. A
        // failed program stays on screen so its error can be read.
        onExited: () => {},
        onAttachLost: () => void bringUp(),
        onTitle: (title) => {
          titles.set(uuid, title);
          sidebar.retitle();
        },
      },
      renderer,
      fontPx,
    );
    void attachView(view, uuid);
    return view;
  }

  function send(op: Op): void {
    connection?.treeOp(op);
  }

  const panes = new Panes(
    parent,
    makeView,
    () => sidebar.markFocused(),
    (tab, split) => send({ type: "set_sizes", tab, split }),
  );
  panes.setFocusFollowsMouse(settings.focusFollowsMouse);
  panes.setDim(settings.dim);

  const sidebar = new Sidebar(sidebarElement, store, {
    select: (tab) => {
      store.select(tab);
      send({ type: "used", tab });
    },
    collapse: (group, collapsed) => {
      // Shown at once, then the host's pushed tree says the same.
      store.setTree(withCollapsed(store.tree, group, collapsed));
      send({ type: "collapse_group", group, collapsed });
    },
    focusPane: (tab, session) => {
      store.select(tab);
      panes.focusSession(session);
    },
    title: (session) => titles.get(session) ?? folders.get(session),
    focusedSession: () => panes.focusedSession(),
    op: send,
    hoverPane: settings.focusFollowsMouse
      ? (tab, session) => {
          if (store.current === tab) panes.focusSession(session);
        }
      : undefined,
    resized: (width) => {
      saveSettings({ sidebarWidth: width }).catch((error: unknown) =>
        bar.note(`Could not save the sidebar width: ${error}`),
      );
    },
  });
  panes.setStateSource((session) => store.stateOf(session));
  sidebar.setWidth(clampWidth(settings.sidebarWidth));
  sidebarElement.hidden = !settings.sidebarShown;

  store.subscribe(() => {
    panes.show(store.currentTab());
    panes.prune(sessionsIn(store.tree), allTabs(store.tree));
    if (store.current !== null) rememberTab(store.current);
    sidebar.render();
    sendSeen();
    if (!focusHeldAside(sidebarElement)) refocus();
  });
  installFocusRule(document.body, panes);
  // States change only markers, in place: no row is built again and no terminal is touched.
  // Every marker that changes, in the sidebar and the panes, goes through one pass: all the
  // reads first, then all the writes.
  store.subscribeStates(() => {
    const changes: MarkerChange[] = [];
    sidebar.updateStates(changes);
    panes.updateStates(changes);
    setMarkerStates(changes);
    sendSeen();
  });

  // Nobody is looking at a done tab while the window is behind another app, and the working
  // tick rests meanwhile (`.win-unfocused` in styles.css). Back in front, every flake restarts
  // in step with the clock, so flakes paused and flakes made later turn together.
  let windowFocused = document.hasFocus();
  document.body.classList.toggle("win-unfocused", !windowFocused);
  window.addEventListener("focus", () => {
    windowFocused = true;
    document.body.classList.remove("win-unfocused");
    resyncTicks(document);
    sendSeen();
  });
  window.addEventListener("blur", () => {
    windowFocused = false;
    document.body.classList.add("win-unfocused");
  });

  /**
   * Tells the host the done sessions on screen have been seen, which turns them idle. A frame the
   * closed socket did not take goes again next time.
   */
  function sendSeen(): void {
    const live = connection;
    if (!live) return;
    store.sendSeen(windowFocused, (session) => live.seen(session));
  }

  function save(patch: Parameters<typeof saveSettings>[0]): void {
    saveSettings(patch).catch((error: unknown) => bar.note(`Could not save the settings: ${error}`));
  }

  const ui = {
    toggleSidebar: () => {
      if (!sidebarElement) return;
      sidebarElement.hidden = !sidebarElement.hidden;
      save({ sidebarShown: !sidebarElement.hidden });
      refocus();
    },
    rename: () => {
      if (!sidebarElement.hidden) sidebar.renameCurrent();
    },
    zoom: (step: 1 | -1 | 0) => {
      const next = nextSize(fontPx, step);
      if (next === fontPx) return;
      fontPx = next;
      applyZoom(panes, next);
      save({ fontSize: next });
    },
    note: (text: string) => bar.note(text),
  };

  function act(action: Action): void {
    if (connection) runAction(action, { store, panes, connection, sessions, home, ui });
  }
  installKeys(DEFAULT_KEYS, act);
  // The menu bar's Close tab item (Cmd+Shift+W): the menu takes the key before the page does.
  if (inTauri()) {
    listen("close-tab", () => act("closeTab")).catch((error: unknown) =>
      bar.note(`The Close tab menu item will not work: ${error}`),
    );
  }

  function onTree(tree: Tree): void {
    const hadTabs = allTabs(store.tree).length > 0;
    store.setTree(tree);
    if (settled && hadTabs && allTabs(tree).length === 0) {
      getCurrentWindow()
        .close()
        .catch((error: unknown) => bar.show(`Could not close the window: ${error}`));
    }
  }

  async function bringUp(): Promise<void> {
    if (bringingUp) return;
    bringingUp = true;
    settled = false;
    // Views from before a reconnect attach again once the new host's numbers are known.
    const existing = panes.sessions();
    // Finding the host starts it if it is gone, so a host that keeps crashing must not be
    // started again every second.
    let retry = RETRY_MS;
    for (;;) {
      connection?.close();
      let port: number | undefined;
      try {
        const host = await findHost();
        port = host.port;
        home = host.home;
        const known = new Sessions();
        const next = new Connection({
          onLost: () => {
            bar.show(LOST_TEXT);
            void bringUp();
          },
          onError: (code, message) => {
            bar.note(
              code === "input_dropped"
                ? `Typing was dropped: ${message}`
                : `The host says: ${message}`,
            );
          },
          onCwd: (session, cwd) => {
            if (connection !== next) return;
            const uuid = known.moved(session, cwd);
            // The sidebar's name for an untitled pane, which the `list` per new tab used to update.
            if (uuid) folders.set(uuid, folderName(cwd));
          },
          onTree: (tree) => {
            if (connection === next) onTree(tree);
          },
          onStates: (items) => {
            if (connection === next) store.setStates(items);
          },
        });
        connection = next;
        sessions = known;
        // The old host may never have read a `seen`, so each done tab is told again.
        store.forgetSeen();
        await next.open(host);
        await refreshSessions(next, known);
        // A new window goes back to the tab it showed last time. Set before the first tree, so
        // no other tab is shown on the way; a tab that is gone gives way to the first one.
        if (store.current === null) store.current = lastTab() ?? null;
        const tree = await next.treeGet();
        if (allTabs(tree).length === 0) {
          const uuid = crypto.randomUUID();
          known.creating(uuid, host.home);
          const session = await next
            .create(uuid, "shell", host.home, START_COLS, START_ROWS, { type: "new_tab" })
            .catch((error: unknown) => {
              known.failed(uuid);
              throw error;
            });
          known.created(uuid, session);
        }
        for (const uuid of existing) {
          const view = panes.viewFor(uuid);
          if (view) void attachView(view, uuid);
        }
        settled = true;
        bar.clear();
        sendSeen();
        bringingUp = false;
        return;
      } catch (error) {
        bar.show(describe(error, port));
        await sleep(retry);
        retry = Math.min(retry * 2, RETRY_MAX_MS);
      }
    }
  }

  bar.show("Looking for the host");
  await bringUp();
}

void main();
