// The sidebar: one scrolling list of ungrouped tabs, then each group's header and tabs. Every
// size and color is in styles.css, taken from docs/ui-review.md.

import type { Group, Tab } from "./connection";
import { refocus } from "./focus";
import { type Row, type Store, leaves } from "./store";

export const MIN_WIDTH = 168;
export const DEFAULT_WIDTH = 240;
export const MAX_WIDTH = 480;
/** The terminal area never gets narrower than this because of the sidebar. */
export const TERMINAL_MIN_WIDTH = 320;

const FLIP_MS = 180;

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
    grip.addEventListener("pointerdown", this.onGripDown);
    window.addEventListener("resize", this.onWindowResize);
    this.applyWidth();
  }

  /** Builds the list again, unless nothing it shows has changed. */
  render(): void {
    const rows = this.store.rows();
    const shape = this.shapeOf(rows);
    if (shape === this.shape) return;
    this.shape = shape;
    this.endFlip();
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
    this.list.replaceChildren(...sections, fill);
    this.rove(this.list.querySelector<HTMLElement>(".row.selected") ?? this.items()[0]);
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
    // The rolled-up blocked marker of a collapsed group goes after the count in a `span.roll`,
    // from the state markers task. It is added only when there is a marker, so without one the
    // count sits at the right edge (mock 1224).
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
    // The state marker goes here, from the state markers task. Empty, it still keeps its width.
    const slot = document.createElement("span");
    slot.className = "slot";
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
    row.append(slot, pidx, name);
    return row;
  }

  private readonly onClick = (event: MouseEvent): void => {
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
    if (row?.dataset.tab) this.events.select(row.dataset.tab);
  };

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
    else if (target.dataset.tab) this.events.select(target.dataset.tab);
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
