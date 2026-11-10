// The sidebar: one scrolling list of ungrouped tabs, then each group's header and tabs. Every
// size and color is in styles.css, taken from docs/ui-review.md.

import type { AgentState, Group, Op, Tab } from "./connection";
import { refocus } from "./focus";
import { type MarkerChange, createMarker, markerState, setMarkerStates, showMarker, syncTick } from "./marker";
import { RENAME_END, renaming, startRename } from "./rename";
import { type Row, type Store, findTab, leaves } from "./store";

export const MIN_WIDTH = 168;
export const DEFAULT_WIDTH = 240;
export const MAX_WIDTH = 480;
/** The terminal area never gets narrower than this because of the sidebar. */
export const TERMINAL_MIN_WIDTH = 320;

const FLIP_MS = 180;
// How far the pointer travels before a press on a row becomes a drag.
const DRAG_START_PX = 4;

// What the keyboard moves between: tab rows and the folder button of each group header.
const ITEMS = ".row, .ghead > .slot";

const FOLDER_SVG =
  '<svg class="gfold" viewBox="0 0 13 13" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M1.5 10V3.2C1.5 2.8 1.8 2.5 2.2 2.5H4.9L6.1 3.8H10.8C11.2 3.8 11.5 4.1 11.5 4.5V10C11.5 10.4 11.2 10.7 10.8 10.7H2.2C1.8 10.7 1.5 10.4 1.5 10Z"/>' +
  '<path class="ff" fill="currentColor" fill-opacity="0.22" d="M1.5 5.4H11.5V10C11.5 10.4 11.2 10.7 10.8 10.7H2.2C1.8 10.7 1.5 10.4 1.5 10Z"/>' +
  "</svg>";

/** Whole pixels between the minimum and maximum. Anything that is not a number gives the default. */
export function clampWidth(px: number): number {
  if (!Number.isFinite(px)) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(px)));
}

/**
 * The width to show: the saved width, narrowed so the terminal area keeps at least 320 px, and
 * never under the minimum.
 */
export function fitWidth(saved: number, windowWidth: number): number {
  return Math.max(MIN_WIDTH, Math.min(clampWidth(saved), Math.floor(windowWidth) - TERMINAL_MIN_WIDTH));
}

function folderName(path: string): string {
  const parts = path.split("/").filter((part) => part !== "");
  return parts[parts.length - 1] ?? path;
}

/**
 * Compact rows are the only kind built so far. Two-line rows (`sidebar_rows: "two"`, R24) need
 * the second line's text, which comes with the state markers.
 */
export type RowStyle = "compact";

type Flip = { tops: Map<string, number>; group: string; clones: HTMLElement[] };

type DropTarget =
  | { kind: "insert"; y: number; op: Op }
  | { kind: "join"; row: HTMLElement; tab: string; into: string };

type Drag = {
  source: HTMLElement;
  startX: number;
  startY: number;
  live: boolean;
  // Esc ended it: nothing shows or drops, and the release is still swallowed.
  cancelled?: boolean;
  ghost?: HTMLElement;
  // Measured once when the drag starts, so moving the pointer reads no layout.
  rects: { element: HTMLElement; top: number; bottom: number }[];
  sidebarTop: number;
  sidebarLeft: number;
  target?: DropTarget;
};

export type SidebarEvents = {
  select(tab: string): void;
  collapse(group: string, collapsed: boolean): void;
  focusPane(tab: string, session: string): void;
  /** The terminal title of a session, else its folder name. */
  title(session: string): string | undefined;
  /** The width after a drag on the edge ends. */
  resized?(width: number): void;
  /** Which pane of the current tab has typing. */
  focusedSession?(): string | undefined;
  /** Rename, pin, close, move and join, straight to the host. */
  op?(op: Op): void;
  /** The pointer came onto a pane row; set only while focus follows the pointer (R31). */
  hoverPane?(tab: string, session: string): void;
};

export class Sidebar {
  readonly element: HTMLElement;
  private readonly store: Store;
  private readonly events: SidebarEvents;
  private readonly list: HTMLElement;
  private shape = "";
  private flipFrom: Flip | undefined;
  private flipTimer = 0;
  private flipLayer: HTMLElement | undefined;
  // The width the user chose, and the width shown, which a narrow window can make smaller.
  private savedWidth = DEFAULT_WIDTH;
  private shownWidth = 0;
  private fitFrame = 0;
  private renaming = false;
  private renderLater = false;
  private menus: HTMLElement[] = [];
  private menuRow: HTMLElement | undefined;
  private drag: Drag | undefined;
  private dragged = false;
  private hovered: HTMLElement | undefined;
  // Stops the working tick of markers scrolled out of the list (absent in tests).
  private ticks: IntersectionObserver | undefined;

  constructor(element: HTMLElement, store: Store, events: SidebarEvents, rowStyle: RowStyle = "compact") {
    this.element = element;
    this.store = store;
    this.events = events;
    element.classList.add("theme-mac", `rows-${rowStyle}`);
    // Only the empty parts move the window: the top strip, the space below the rows and the
    // usage line. Rows and the scrollbar are left alone.
    element.setAttribute("data-tauri-drag-region", "");
    this.list = document.createElement("div");
    this.list.id = "list";
    const usage = document.createElement("div");
    usage.id = "usage";
    usage.setAttribute("data-tauri-drag-region", "");
    const grip = document.createElement("div");
    grip.className = "grip";
    element.replaceChildren(this.list, usage, grip);
    this.list.addEventListener("click", this.onClick);
    // Keys reach these only while a row or header has focus, never on the terminal's key path.
    this.list.addEventListener("keydown", this.onKeyDown);
    this.list.addEventListener("focusin", this.onFocusIn);
    this.list.addEventListener("dblclick", this.onDoubleClick);
    this.list.addEventListener("contextmenu", this.onContextMenu);
    this.list.addEventListener("pointerdown", this.onPointerDown);
    this.list.addEventListener("mouseover", this.onMouseOver);
    this.list.addEventListener("pointerleave", this.onPointerLeave);
    this.list.addEventListener(RENAME_END, this.onRenameEnd);
    grip.addEventListener("pointerdown", this.onGripDown);
    window.addEventListener("resize", this.onWindowResize);
    if (typeof IntersectionObserver !== "undefined") {
      this.ticks = new IntersectionObserver(
        (entries) => {
          // A marker back in view starts its tick again from 0, so it takes the clock's phase,
          // one for the whole callback. One that never left keeps its running tick.
          const now = performance.now();
          for (const entry of entries) {
            const mk = entry.target as HTMLElement;
            const back = entry.isIntersecting && mk.classList.contains("off");
            mk.classList.toggle("off", !entry.isIntersecting);
            if (back) syncTick(mk, now);
          }
        },
        { root: this.list },
      );
    }
    this.applyWidth();
  }

  /** Builds the list again, unless nothing it shows has changed. */
  render(): void {
    // A rebuild would take the rename field away mid-word; it runs once the field closes.
    if (this.renaming) {
      this.renderLater = true;
      return;
    }
    const rows = this.store.rows();
    const shape = this.shapeOf(rows);
    if (shape === this.shape) return;
    this.shape = shape;
    this.endFlip();
    this.closeMenu();
    this.endDrag();
    // Keyboard focus on a row or header stays on the same one in the new list.
    const active = document.activeElement;
    const kept =
      active instanceof HTMLElement && active.matches(ITEMS) && this.list.contains(active)
        ? `${active.closest<HTMLElement>("[data-key]")?.dataset.key} ${active.classList.contains("slot")}`
        : undefined;
    const from = this.flipFrom;
    this.flipFrom = undefined;

    const sections: HTMLElement[] = [];
    let section: HTMLElement | undefined;
    let panes: HTMLElement | undefined;
    let paneIndex = 0;
    for (const row of rows) {
      if (row.kind === "group") {
        section = document.createElement("section");
        section.className = row.group.collapsed ? "group collapsed" : "group";
        section.append(this.header(row.group));
        sections.push(section);
        panes = undefined;
      } else if (row.kind === "tab") {
        if (!section) {
          section = document.createElement("div");
          section.className = "loose";
          sections.push(section);
        }
        section.append(this.tabRow(row.tab));
        panes = undefined;
      } else {
        if (!panes) {
          panes = document.createElement("div");
          panes.className = "panes";
          section?.append(panes);
          paneIndex = 0;
        }
        panes.append(this.paneRow(row.session, row.tab, ++paneIndex));
      }
    }
    const fill = document.createElement("div");
    fill.className = "fill";
    fill.setAttribute("data-tauri-drag-region", "");
    this.ticks?.disconnect();
    this.list.replaceChildren(...sections, fill);
    for (const mk of this.list.querySelectorAll<HTMLElement>(".mk")) this.ticks?.observe(mk);
    this.rove(this.list.querySelector<HTMLElement>(".row.selected") ?? this.items()[0]);
    if (kept !== undefined) {
      this.items()
        .find((item) => `${item.closest<HTMLElement>("[data-key]")?.dataset.key} ${item.classList.contains("slot")}` === kept)
        ?.focus();
    }
    this.markFocused();
    if (from) this.playFlip(from);
  }

  /** Puts new terminal titles on unnamed rows and pane rows, without building the list again. */
  retitle(): void {
    for (const name of this.list.querySelectorAll<HTMLElement>("[data-auto]")) {
      const text = this.autoName(name.dataset.auto ?? "", name.dataset.folder);
      if (name.textContent !== text) {
        name.textContent = text;
        name.closest(".row")?.setAttribute("aria-label", text);
      }
    }
    for (const row of this.list.querySelectorAll<HTMLElement>(".row[data-tab]")) {
      const tab = findTab(this.store.tree, row.dataset.tab ?? "");
      if (tab) this.applySub(row, tab, this.store.tabState(tab));
    }
  }

  /**
   * Shows new session states on the markers already in the list, in place, so each one morphs.
   * Builds no row. The markers that change go through one `setMarkerStates`, so the list is
   * styled and laid out once for all of them; with `into` they are added to it instead, for the
   * caller to make together with the panes' corner markers.
   */
  updateStates(into?: MarkerChange[]): void {
    const changes = into ?? [];
    for (const row of this.list.querySelectorAll<HTMLElement>(".row[data-tab]")) {
      const tab = findTab(this.store.tree, row.dataset.tab ?? "");
      const slot = row.querySelector<HTMLElement>(":scope > .slot");
      if (!tab || !slot) continue;
      const state = this.store.tabState(tab);
      this.place(slot, state, changes);
      this.applySub(row, tab, state);
    }
    for (const row of this.list.querySelectorAll<HTMLElement>(".pane-row")) {
      const slot = row.querySelector<HTMLElement>(":scope > .slot");
      if (slot) this.place(slot, this.store.stateOf(row.dataset.session ?? ""), changes);
    }
    for (const head of this.list.querySelectorAll<HTMLElement>(".ghead")) {
      const group = this.store.tree.groups.find((g) => g.key === head.dataset.group);
      if (group) this.applyRoll(head, group);
    }
    if (!into) setMarkerStates(changes);
  }

  // Shows `state` in a row's slot. The tick observer follows a marker made and lets go of one
  // taken away.
  private place(slot: HTMLElement, state: AgentState | null, changes: MarkerChange[]): void {
    const old = slot.querySelector<HTMLElement>(":scope > .mk");
    const mk = showMarker(slot, state, changes);
    if (old && old !== mk) this.ticks?.unobserve(old);
    if (mk && mk !== old) this.ticks?.observe(mk);
  }

  // A collapsed group with a blocked session shows the blocked marker after its count (mock
  // 1224). Without one there is no roll slot, so the count sits at the right edge.
  private applyRoll(head: HTMLElement, group: Group): void {
    const want = group.collapsed && this.store.groupBlocked(group);
    const roll = head.querySelector(":scope > .roll");
    if (want && !roll) {
      const made = document.createElement("span");
      made.className = "roll";
      made.title = "A session in this group is waiting on you";
      made.append(createMarker("blocked"));
      head.append(made);
    } else if (!want && roll) {
      const mk = roll.querySelector<HTMLElement>(".mk");
      if (mk) this.ticks?.unobserve(mk);
      roll.remove();
    }
  }

  // A blocked or done row gets a dim second line with the title of the session in that state,
  // even when that is also the row's name (an unnamed tab), so every such row reads the same.
  // A blank title counts as none, and the line shows the row's name.
  private applySub(row: HTMLElement, tab: Tab, state: AgentState | null): void {
    const body = row.querySelector<HTMLElement>(":scope > .body");
    if (!body) return;
    let text = "";
    if (state === "blocked" || state === "done") {
      const session = leaves(tab.split).find((s) => this.store.stateOf(s) === state);
      const title = session !== undefined ? this.events.title(session) : undefined;
      text = (title?.trim() ? title : body.querySelector(".name")?.textContent) ?? "";
    }
    let sub = body.querySelector<HTMLElement>(":scope > .sub");
    if (text === "") {
      sub?.remove();
      row.classList.remove("has-sub");
      return;
    }
    if (!sub) {
      sub = document.createElement("span");
      sub.className = "sub";
      body.append(sub);
      row.classList.add("has-sub");
    }
    if (sub.textContent !== text) sub.textContent = text;
  }

  /** Opens the rename field on the selected tab's row (Cmd+Shift+R). */
  renameCurrent(): void {
    const row = this.list.querySelector<HTMLElement>(".row.selected");
    if (row) this.renameRow(row);
  }

  /** Marks the sidebar pane row of the pane that has typing. */
  markFocused(): void {
    const session = this.events.focusedSession?.();
    for (const row of this.list.querySelectorAll<HTMLElement>(".pane-row")) {
      row.classList.toggle("focused", row.dataset.session === session);
    }
  }

  /** Saves `width` as the chosen width and shows as much of it as the window leaves room for. */
  setWidth(width: number): void {
    this.savedWidth = clampWidth(width);
    this.applyWidth();
  }

  private applyWidth(): void {
    const width = fitWidth(this.savedWidth, window.innerWidth);
    if (width === this.shownWidth) return;
    this.shownWidth = width;
    this.element.style.width = `${width}px`;
  }

  // At most once a frame, and it reads only the window's width, so it never forces a layout.
  private readonly onWindowResize = (): void => {
    if (this.fitFrame) return;
    this.fitFrame = requestAnimationFrame(() => {
      this.fitFrame = 0;
      this.applyWidth();
    });
  };

  private items(): HTMLElement[] {
    return [...this.list.querySelectorAll<HTMLElement>(ITEMS)];
  }

  // One tab stop in the list: `current` takes it, every other row and header gives it up.
  private rove(current: HTMLElement | undefined): void {
    for (const item of this.items()) item.tabIndex = item === current ? 0 : -1;
  }

  // Everything the list shows, so a pushed tree that only moved `last_used` costs nothing.
  private shapeOf(rows: Row[]): string {
    const parts = rows.map((row) => {
      switch (row.kind) {
        case "group": {
          const g = row.group;
          return `g\0${g.key}\0${g.name}\0${g.path}\0${g.branch}\0${g.collapsed}\0${this.count(g)}`;
        }
        case "tab":
          return `t\0${row.tab.id}\0${row.tab.name}\0${leaves(row.tab.split)[0]}`;
        case "pane":
          return `p\0${row.session}`;
      }
    });
    return `${this.store.current}\n${parts.join("\n")}`;
  }

  private count(group: Group): number {
    return group.tabs.reduce((sum, tab) => sum + leaves(tab.split).length, 0);
  }

  private autoName(session: string, folder: string | undefined): string {
    return this.events.title(session) ?? folder ?? "Shell";
  }

  private header(group: Group): HTMLElement {
    const name = group.name ?? folderName(group.path);
    const head = document.createElement("div");
    head.className = "ghead";
    head.dataset.key = `group:${group.key}`;
    head.dataset.group = group.key;

    const fold = document.createElement("button");
    fold.type = "button";
    fold.className = "slot";
    fold.setAttribute("aria-expanded", String(!group.collapsed));
    fold.setAttribute("aria-label", `${group.collapsed ? "Expand" : "Collapse"} ${name}`);
    fold.innerHTML = FOLDER_SVG;

    const gname = document.createElement("span");
    gname.className = "gname";
    gname.textContent = name;
    head.append(fold, gname);
    if (group.branch) {
      const branch = document.createElement("span");
      branch.className = "branch";
      branch.textContent = group.branch;
      head.append(branch);
    }
    const count = document.createElement("span");
    count.className = "gcount";
    count.textContent = String(this.count(group));
    head.append(count);
    this.applyRoll(head, group);
    return head;
  }

  private tabRow(tab: Tab): HTMLElement {
    const row = document.createElement("div");
    const selected = tab.id === this.store.current;
    row.className = selected ? "row selected" : "row";
    row.dataset.key = `tab:${tab.id}`;
    row.dataset.tab = tab.id;
    row.setAttribute("role", "button");
    if (selected) row.setAttribute("aria-current", "true");
    // The state marker. Empty, the slot still keeps its width.
    const slot = document.createElement("span");
    slot.className = "slot";
    const state = this.store.tabState(tab);
    showMarker(slot, state);
    const body = document.createElement("span");
    body.className = "body";
    const name = document.createElement("span");
    name.className = "name";
    if (tab.name !== null) {
      name.textContent = tab.name;
    } else {
      const session = leaves(tab.split)[0] ?? "";
      const group = this.store.tree.groups.find((g) => g.tabs.includes(tab));
      name.dataset.auto = session;
      if (group) name.dataset.folder = folderName(group.path);
      name.textContent = this.autoName(session, name.dataset.folder);
    }
    body.append(name);
    row.setAttribute("aria-label", name.textContent ?? "");
    row.append(slot, body);
    this.applySub(row, tab, state);
    return row;
  }

  private paneRow(session: string, tab: string, index: number): HTMLElement {
    const row = document.createElement("div");
    row.className = "pane-row";
    row.dataset.key = `pane:${session}`;
    row.dataset.session = session;
    row.dataset.tab = tab;
    const slot = document.createElement("span");
    slot.className = "slot";
    const pidx = document.createElement("span");
    pidx.className = "pidx";
    pidx.textContent = String(index);
    const name = document.createElement("span");
    name.className = "pname";
    name.dataset.auto = session;
    name.textContent = this.autoName(session, undefined);
    showMarker(slot, this.store.stateOf(session));
    row.append(slot, pidx, name);
    return row;
  }

  private readonly onClick = (event: MouseEvent): void => {
    // The click that ends a drag is not a click on the row.
    if (this.dragged) {
      this.dragged = false;
      return;
    }
    const target = event.target instanceof Element ? event.target : null;
    const fold = target?.closest<HTMLElement>(".ghead .slot");
    if (fold) {
      this.toggle(fold);
      return;
    }
    const pane = target?.closest<HTMLElement>(".pane-row");
    if (pane?.dataset.session && pane.dataset.tab) {
      this.events.focusPane(pane.dataset.tab, pane.dataset.session);
      return;
    }
    const row = target?.closest<HTMLElement>(".row");
    if (row?.dataset.tab) this.pick(row.dataset.tab);
  };

  // Only a real change of tab is a use, so a double click sends one `used`, not two.
  private pick(tab: string): void {
    if (tab !== this.store.current) this.events.select(tab);
  }

  private renameRow(row: HTMLElement): void {
    const tab = row.dataset.tab ? findTab(this.store.tree, row.dataset.tab) : undefined;
    if (!tab || this.renaming) return;
    this.renaming = true;
    startRename(row, row.querySelector(".name")?.textContent ?? "", (name) => {
      this.events.op?.({ type: "rename_tab", tab: tab.id, name });
    });
  }

  private renameGroup(head: HTMLElement): void {
    const group = this.store.tree.groups.find((g) => g.key === head.dataset.group);
    if (!group || this.renaming) return;
    this.renaming = true;
    startRename(head, head.querySelector(".gname")?.textContent ?? "", (name) => {
      this.events.op?.({ type: "rename_group", group: group.key, name });
    });
  }

  private readonly onRenameEnd = (): void => {
    this.renaming = false;
    // The pointer may still be on a pane row, where no mouseover comes until it leaves: the
    // next move over the list counts as pointing at the row under it. Once, so moves cost
    // nothing after that.
    this.hovered = undefined;
    if (this.events.hoverPane) this.list.addEventListener("mousemove", this.onMouseOver, { once: true });
    if (!this.renderLater) return;
    this.renderLater = false;
    this.render();
  };

  // A double click on a tab row or a group name renames it. Not on the folder button, which
  // opens and closes the group.
  private readonly onDoubleClick = (event: MouseEvent): void => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target || target.closest(".slot, input")) return;
    const row = target.closest<HTMLElement>(".row");
    if (row) {
      this.renameRow(row);
      return;
    }
    const head = target.closest<HTMLElement>(".ghead");
    if (head) this.renameGroup(head);
  };

  private readonly onMouseOver = (event: MouseEvent): void => {
    const target = event.target instanceof Element ? event.target : null;
    const pane = target?.closest<HTMLElement>(".pane-row") ?? undefined;
    // A rename field keeps typing until Enter, Esc or a click elsewhere. Nothing is recorded, so
    // the row under the pointer still counts once the field closes.
    if (renaming()) {
      this.hovered = undefined;
      return;
    }
    if (pane === this.hovered) return;
    this.hovered = pane;
    if (pane?.dataset.tab && pane.dataset.session) this.events.hoverPane?.(pane.dataset.tab, pane.dataset.session);
  };

  // Out of the list the pointer may give focus to a terminal pane, so coming back to the same
  // row has to count again. Only forgets the row; focus moves on the next mouseover.
  private readonly onPointerLeave = (): void => {
    this.hovered = undefined;
  };

  // Right-click menu on a tab row: four items (docs/ui-review.md "Right-click menu"). It sits
  // outside the sidebar, whose paint containment would clip a 200 px menu.

  private readonly onContextMenu = (event: MouseEvent): void => {
    const target = event.target instanceof Element ? event.target : null;
    const row = target?.closest<HTMLElement>(".row");
    const tab = row?.dataset.tab ? findTab(this.store.tree, row.dataset.tab) : undefined;
    if (!row || !tab) return;
    event.preventDefault();
    this.openMenu(row, tab);
  };

  private openMenu(row: HTMLElement, tab: Tab): void {
    this.closeMenu();
    const groupOf = this.store.tree.groups.find((g) => g.tabs.some((t) => t.id === tab.id));
    const targets: { label: string; group: string | null; size: number }[] = [];
    if (groupOf) targets.push({ label: "No group", group: null, size: this.store.tree.ungrouped.length });
    for (const g of this.store.tree.groups) {
      if (g !== groupOf) targets.push({ label: g.name ?? folderName(g.path), group: g.key, size: g.tabs.length });
    }
    const menu = this.menu([
      { label: "Rename", hint: "\u21E7\u2318R", run: () => this.renameRow(row) },
      {
        label: tab.pinned ? "Unpin" : "Pin",
        hint: "",
        run: () => this.events.op?.({ type: "pin_tab", tab: tab.id, pinned: !tab.pinned }),
      },
      { label: "Close", hint: "\u21E7\u2318W", run: () => this.events.op?.({ type: "close_tab", tab: tab.id }) },
      {
        label: "Move to group",
        hint: "\u203A",
        disabled: targets.length === 0,
        open: (item) =>
          this.menu(
            targets.map((target) => ({
              label: target.label,
              hint: "",
              run: () =>
                this.events.op?.({ type: "move_tab", tab: tab.id, group: target.group, index: target.size }),
            })),
            item,
          ),
      },
    ]);
    const rect = row.getBoundingClientRect();
    const side = this.element.getBoundingClientRect();
    menu.style.left = `${side.left + Math.min(72, this.shownWidth - 60)}px`;
    menu.style.top = `${Math.max(0, Math.min(rect.bottom - 2, window.innerHeight - menu.offsetHeight - 4))}px`;
    row.classList.add("menu-open");
    this.menuRow = row;
    window.addEventListener("pointerdown", this.onMenuOutside, true);
    window.addEventListener("keydown", this.onMenuKey, true);
    window.addEventListener("blur", this.closeMenu);
  }

  /** A menu of `items`, in the page. A submenu opens beside `beside`, its item in the menu above. */
  private menu(
    items: {
      label: string;
      hint: string;
      disabled?: boolean;
      run?: () => void;
      open?: (item: HTMLElement) => HTMLElement;
    }[],
    beside?: HTMLElement,
  ): HTMLElement {
    const menu = document.createElement("div");
    menu.className = "ctx-menu";
    menu.setAttribute("role", "menu");
    // The menu above keeps only this one submenu.
    const depth = beside ? this.menus.indexOf(beside.closest<HTMLElement>(".ctx-menu") as HTMLElement) + 1 : 0;
    for (const extra of this.menus.splice(depth)) extra.remove();
    for (const entry of items) {
      const item = document.createElement("div");
      item.className = entry.disabled ? "item disabled" : "item";
      item.setAttribute("role", "menuitem");
      const label = document.createElement("span");
      label.className = "label";
      label.textContent = entry.label;
      const hint = document.createElement("span");
      hint.className = "hint";
      hint.textContent = entry.hint;
      item.append(label, hint);
      if (!entry.disabled) {
        if (entry.open) {
          const open = entry.open;
          const show = () => {
            if (!this.menus.some((m) => m.dataset.parent === entry.label)) open(item).dataset.parent = entry.label;
          };
          item.addEventListener("mouseenter", show);
          item.addEventListener("click", show);
        } else {
          const run = entry.run;
          item.addEventListener("mouseenter", () => {
            for (const extra of this.menus.splice(depth + 1)) extra.remove();
          });
          item.addEventListener("click", () => {
            this.closeMenu();
            run?.();
          });
        }
      }
      menu.append(item);
    }
    document.body.append(menu);
    this.menus.push(menu);
    if (beside) {
      const rect = beside.getBoundingClientRect();
      menu.style.left = `${Math.min(rect.right + 2, window.innerWidth - menu.offsetWidth - 4)}px`;
      menu.style.top = `${Math.max(0, Math.min(rect.top - 4, window.innerHeight - menu.offsetHeight - 4))}px`;
    }
    return menu;
  }

  private readonly closeMenu = (): void => {
    if (this.menus.length === 0) return;
    for (const menu of this.menus) menu.remove();
    this.menus = [];
    this.menuRow?.classList.remove("menu-open");
    this.menuRow = undefined;
    window.removeEventListener("pointerdown", this.onMenuOutside, true);
    window.removeEventListener("keydown", this.onMenuKey, true);
    window.removeEventListener("blur", this.closeMenu);
  };

  private readonly onMenuOutside = (event: PointerEvent): void => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.closest(".ctx-menu")) this.closeMenu();
  };

  // Only while a menu is open: Esc closes it and never reaches the terminal.
  private readonly onMenuKey = (event: KeyboardEvent): void => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    this.closeMenu();
  };

  // Dragging rows (docs/ui-review.md "Drag targets"). A tab row moves within its own list or
  // joins another tab as a split (right, or down with Shift); a group header moves between
  // groups; a pane row dropped in a gap between rows becomes its own tab.

  private readonly onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0 || this.renaming) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target || target.closest("input")) return;
    const source = target.closest<HTMLElement>(".row, .ghead, .pane-row");
    if (!source) return;
    this.endDrag();
    this.drag = {
      source,
      startX: event.clientX,
      startY: event.clientY,
      live: false,
      rects: [],
      sidebarTop: 0,
      sidebarLeft: 0,
    };
    window.addEventListener("pointermove", this.onDragMove);
    window.addEventListener("pointerup", this.onDragUp);
    window.addEventListener("pointercancel", this.endDrag);
  };

  private readonly onDragMove = (event: PointerEvent): void => {
    const drag = this.drag;
    if (!drag || drag.cancelled) return;
    if (!drag.live) {
      if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < DRAG_START_PX) return;
      this.beginDrag(drag);
    }
    if (drag.ghost) {
      drag.ghost.style.transform = `translate(${event.clientX - drag.sidebarLeft + 8}px, ${event.clientY - drag.sidebarTop - drag.ghost.offsetHeight / 2}px)`;
    }
    this.showTarget(drag, this.targetAt(drag, event.clientY));
  };

  private beginDrag(drag: Drag): void {
    drag.live = true;
    this.closeMenu();
    window.addEventListener("keydown", this.onDragKey, true);
    const side = this.element.getBoundingClientRect();
    drag.sidebarTop = side.top;
    drag.sidebarLeft = side.left;
    drag.rects = [...this.list.querySelectorAll<HTMLElement>(".row, .ghead")].map((element) => {
      const rect = element.getBoundingClientRect();
      return { element, top: rect.top, bottom: rect.bottom };
    });
    drag.source.classList.add("drag-src");
    const ghost = document.createElement("div");
    ghost.className = "ghost";
    const slot = document.createElement("span");
    slot.className = "slot";
    const shown = drag.source.querySelector(":scope > .slot > .mk");
    const state = shown ? markerState(shown) : null;
    if (state) slot.append(createMarker(state));
    const name = document.createElement("span");
    name.className = "gtext";
    name.textContent =
      drag.source.querySelector(".name, .gname, .pname")?.textContent ?? "";
    ghost.append(slot, name);
    this.element.append(ghost);
    drag.ghost = ghost;
  }

  // What a drop at `y` would do. Rows are measured, so this reads no layout.
  private targetAt(drag: Drag, y: number): DropTarget | undefined {
    const hit = drag.rects.find((rect) => y >= rect.top && y < rect.bottom);
    if (!hit || hit.element === drag.source) return undefined;
    const { element } = hit;
    const height = hit.bottom - hit.top;
    const after = y >= hit.top + height / 2;
    const gapY = after ? hit.bottom : hit.top;
    const source = drag.source;

    if (source.classList.contains("ghead")) {
      if (!element.classList.contains("ghead")) return undefined;
      const order = this.store.tree.groups.map((g) => g.key).filter((key) => key !== source.dataset.group);
      const index = order.indexOf(element.dataset.group ?? "") + (after ? 1 : 0);
      if (!source.dataset.group || index < 0) return undefined;
      return { kind: "insert", y: gapY, op: { type: "move_group", group: source.dataset.group, index } };
    }

    if (source.classList.contains("pane-row")) {
      // Its own tab's row is where it already is.
      if (element.dataset.tab === source.dataset.tab || !source.dataset.session) return undefined;
      return { kind: "insert", y: gapY, op: { type: "split_out", session: source.dataset.session } };
    }

    const tab = source.dataset.tab;
    if (!tab || !element.classList.contains("row") || !element.dataset.tab) return undefined;
    const middle = y >= hit.top + height / 4 && y < hit.bottom - height / 4;
    if (middle) return { kind: "join", row: element, tab, into: element.dataset.tab };
    // Reordering stays inside the row's own list: its group, or the ungrouped tabs.
    const list = source.parentElement;
    if (element.parentElement !== list || !list) return undefined;
    const order = [...list.children].filter((child) => child.classList.contains("row") && child !== source);
    const index = order.indexOf(element) + (after ? 1 : 0);
    const group = list.classList.contains("group")
      ? (list.querySelector<HTMLElement>(".ghead")?.dataset.group ?? null)
      : null;
    return { kind: "insert", y: gapY, op: { type: "move_tab", tab, group, index } };
  }

  private showTarget(drag: Drag, target: DropTarget | undefined): void {
    const old = drag.target;
    drag.target = target;
    if (old?.kind === "join" && (target?.kind !== "join" || target.row !== old.row)) {
      old.row.classList.remove("join-target");
      old.row.querySelector(".join-tag")?.remove();
    }
    if (target?.kind === "join" && !target.row.classList.contains("join-target")) {
      target.row.classList.add("join-target");
      const tag = document.createElement("span");
      tag.className = "join-tag";
      tag.textContent = "join";
      target.row.append(tag);
    }
    let line = this.element.querySelector<HTMLElement>(":scope > .insert");
    if (target?.kind !== "insert") {
      line?.remove();
      return;
    }
    if (!line) {
      line = document.createElement("div");
      line.className = "insert";
      this.element.append(line);
    }
    line.style.transform = `translateY(${target.y - drag.sidebarTop - 1}px)`;
  }

  private readonly onDragUp = (event: PointerEvent): void => {
    const drag = this.drag;
    const target = drag?.live && !drag.cancelled ? drag.target : undefined;
    this.dragged = drag?.live ?? false;
    this.endDrag();
    if (target?.kind === "join") {
      this.events.op?.({ type: "join_tabs", tab: target.tab, into: target.into, dir: event.shiftKey ? "down" : "right" });
    } else if (target) {
      this.events.op?.(target.op);
    }
    // A click follows only when the press and release were on the same row; any other leaves
    // the flag set, so it is cleared after this event.
    if (this.dragged) setTimeout(() => (this.dragged = false), 0);
  };

  private readonly endDrag = (): void => {
    const drag = this.drag;
    if (!drag) return;
    this.drag = undefined;
    window.removeEventListener("pointermove", this.onDragMove);
    window.removeEventListener("pointerup", this.onDragUp);
    window.removeEventListener("pointercancel", this.endDrag);
    this.clearDrag(drag);
  };

  // Only while a drag is live: Esc ends it where it is, drops nothing, and never reaches the
  // terminal. The press is still followed to its release, so the release clicks no row.
  private readonly onDragKey = (event: KeyboardEvent): void => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    const drag = this.drag;
    if (!drag) return;
    drag.cancelled = true;
    this.clearDrag(drag);
  };

  private clearDrag(drag: Drag): void {
    window.removeEventListener("keydown", this.onDragKey, true);
    drag.source.classList.remove("drag-src");
    drag.ghost?.remove();
    drag.ghost = undefined;
    this.showTarget(drag, undefined);
  }

  private toggle(fold: HTMLElement): void {
    const key = fold.parentElement?.dataset.group;
    const group = this.store.tree.groups.find((g) => g.key === key);
    if (!group) return;
    this.flipFrom = this.measure(group);
    this.events.collapse(group.key, !group.collapsed);
  }

  // Up and Down move between visible rows and headers, Enter or Space selects a tab or opens and
  // closes a group, then typing goes back to the terminal (mock 1844-1853).
  private readonly onKeyDown = (event: KeyboardEvent): void => {
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (!target?.matches(ITEMS) || event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const items = this.items();
      items[items.indexOf(target) + (event.key === "ArrowDown" ? 1 : -1)]?.focus();
      return;
    }
    if (event.key !== "Enter" && event.key !== " ") return;
    // Also keeps the folder button's own click from toggling a second time.
    event.preventDefault();
    if (target.classList.contains("slot")) this.toggle(target);
    else if (target.dataset.tab) this.pick(target.dataset.tab);
    refocus();
  };

  private readonly onFocusIn = (event: FocusEvent): void => {
    const target = event.target instanceof HTMLElement ? event.target : null;
    if (target?.matches(ITEMS)) this.rove(target);
  };

  // Collapse motion, FLIP: measure, build the new list, move each row back to where it was with
  // no transition, then let it slide to its new place. Transform and opacity only.

  private measure(group: Group): Flip | undefined {
    if (typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches) return undefined;
    const tops = new Map<string, number>();
    for (const element of this.list.querySelectorAll<HTMLElement>("[data-key]")) {
      tops.set(element.dataset.key ?? "", element.getBoundingClientRect().top);
    }
    // The rows of a closing group leave as copies, since the new list has no place for them.
    const clones: HTMLElement[] = [];
    if (!group.collapsed) {
      const listTop = this.list.getBoundingClientRect().top;
      const head = this.headOf(group.key);
      for (const element of head?.parentElement?.querySelectorAll<HTMLElement>(".row, .panes") ?? []) {
        const clone = element.cloneNode(true) as HTMLElement;
        clone.style.top = `${element.getBoundingClientRect().top - listTop}px`;
        clones.push(clone);
      }
    }
    return { tops, group: group.key, clones };
  }

  private headOf(key: string): HTMLElement | undefined {
    return [...this.list.querySelectorAll<HTMLElement>(".ghead")].find((head) => head.dataset.group === key);
  }

  private playFlip(from: Flip): void {
    const head = this.headOf(from.group);
    const headOld = from.tops.get(`group:${from.group}`);
    const headShift = head && headOld !== undefined ? headOld - head.getBoundingClientRect().top : 0;
    const moved: HTMLElement[] = [];
    for (const element of this.list.querySelectorAll<HTMLElement>("[data-key]")) {
      const old = from.tops.get(element.dataset.key ?? "");
      if (old === undefined) {
        element.style.transform = `translateY(${-headShift - 4}px)`;
        element.style.opacity = "0";
      } else {
        const dy = old - element.getBoundingClientRect().top;
        if (dy === 0) continue;
        element.style.transform = `translateY(${dy}px)`;
      }
      moved.push(element);
    }
    // The folder flap starts from its old pose too.
    const flap = head?.querySelector<SVGElement>(".ff");
    if (flap) {
      flap.style.transition = "none";
      flap.style.transform = head?.parentElement?.classList.contains("collapsed")
        ? "skewX(-22deg) scaleY(0.7)"
        : "none";
    }
    if (from.clones.length > 0) {
      const layer = document.createElement("div");
      layer.className = "flip-out";
      layer.style.top = `${this.list.offsetTop}px`;
      layer.style.height = `${this.list.clientHeight}px`;
      layer.append(...from.clones);
      this.element.append(layer);
      this.flipLayer = layer;
    }
    void this.list.offsetHeight;
    this.list.classList.add("flip-run");
    this.flipLayer?.classList.add("flip-run");
    for (const element of moved) {
      element.style.transform = "";
      element.style.opacity = "";
    }
    if (flap) {
      flap.style.transition = "";
      flap.style.transform = "";
    }
    for (const clone of from.clones) {
      clone.style.transform = `translateY(${headShift}px)`;
      clone.style.opacity = "0";
    }
    this.flipTimer = window.setTimeout(() => this.endFlip(), FLIP_MS + 80);
  }

  // Before every render too, so an interrupted run never leaves a transform behind.
  private endFlip(): void {
    clearTimeout(this.flipTimer);
    this.flipTimer = 0;
    this.list.classList.remove("flip-run");
    this.flipLayer?.remove();
    this.flipLayer = undefined;
    for (const element of this.list.querySelectorAll<HTMLElement>("[data-key]")) {
      element.style.transform = "";
      element.style.opacity = "";
    }
  }

  private readonly onGripDown = (event: PointerEvent): void => {
    if (event.button !== 0) return;
    event.preventDefault();
    const grip = event.currentTarget as HTMLElement;
    grip.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    const startWidth = this.shownWidth;
    let width = startWidth;
    // Saves what the user sees, so a narrow window never stores a width larger than shown.
    const move = (moveEvent: PointerEvent) => {
      const next = fitWidth(startWidth + moveEvent.clientX - startX, window.innerWidth);
      if (next === width) return;
      width = next;
      this.setWidth(width);
    };
    const up = () => {
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", up);
      grip.removeEventListener("pointercancel", up);
      if (width !== startWidth) this.events.resized?.(width);
      // The preventDefault above also stops the mouseup the focus rule listens for.
      refocus();
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
    grip.addEventListener("pointercancel", up);
  };
}
