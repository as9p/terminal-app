// The terminal area. One TerminalView per session, made the first time its tab is shown and
// kept, hidden, while another tab is on screen. A tab's split becomes nested flex boxes.

import type { Split, Tab } from "./connection";
import { leaves, roundToCells, withSizesAt } from "./store";
import type { TerminalView } from "./terminal";

/** Makes the view for `session` inside `container`, which is already in the page. */
export type MakeView = (container: HTMLElement, session: string) => TerminalView;

/** A divider drag ended: the tab's split with the new sizes, for `set_sizes`. */
export type SizesChanged = (tab: string, split: Split) => void;

export type PaneDirection = "left" | "right" | "up" | "down";

type Entry = { element: HTMLElement; view: TerminalView | undefined };

type Box = { left: number; right: number; top: number; bottom: number };

/**
 * The pane to move to from `from` in `direction`: of those wholly on that side and level with it,
 * the nearest, and of equally near ones the one sharing the most edge.
 */
export function pickNeighbor<K>(from: Box, others: [K, Box][], direction: PaneDirection): K | undefined {
  let best: K | undefined;
  let bestGap = Infinity;
  let bestOverlap = 0;
  const across = direction === "left" || direction === "right";
  for (const [key, box] of others) {
    const gap =
      direction === "right"
        ? box.left - from.right
        : direction === "left"
          ? from.left - box.right
          : direction === "down"
            ? box.top - from.bottom
            : from.top - box.bottom;
    if (gap < -1) continue;
    const overlap = across
      ? Math.min(from.bottom, box.bottom) - Math.max(from.top, box.top)
      : Math.min(from.right, box.right) - Math.max(from.left, box.left);
    if (overlap <= 0) continue;
    if (gap < bestGap - 0.5 || (Math.abs(gap - bestGap) <= 0.5 && overlap > bestOverlap)) {
      best = key;
      bestGap = gap;
      bestOverlap = overlap;
    }
  }
  return best;
}

// The split without its sizes, so a drag that only moved a divider keeps every box in place.
function shapeOf(split: Split): string {
  return JSON.stringify(split, (key, value: unknown) => (key === "sizes" ? undefined : value));
}

export class Panes {
  private readonly root: HTMLElement;
  private readonly make: MakeView;
  private readonly onFocus: () => void;
  private readonly onSizes: SizesChanged;
  private readonly entries = new Map<string, Entry>();
  // Hidden views wait here. display: none, so they neither draw nor take layout work.
  private readonly parked: HTMLElement;
  private tab: Tab | undefined;
  private shownKey = "";
  private shownShape = "";
  private layout: HTMLElement | undefined;
  // The pane typing last went to, per tab, so coming back to a tab lands in the same pane.
  private readonly focused = new Map<string, string>();
  // One pane filling its tab (Cmd+Shift+Enter), client only.
  private filled: { tab: string; session: string } | undefined;
  private followMouse = false;
  private dim = false;

  constructor(
    root: HTMLElement,
    make: MakeView,
    onFocus: () => void = () => {},
    onSizes: SizesChanged = () => {},
  ) {
    this.root = root;
    this.make = make;
    this.onFocus = onFocus;
    this.onSizes = onSizes;
    this.parked = document.createElement("div");
    this.parked.className = "parked";
    this.root.append(this.parked);
  }

  /** Puts `tab` on screen. The same tab with the same split changes nothing in the page. */
  show(tab: Tab | undefined): void {
    this.tab = tab;
    if (this.filled && (!tab || tab.id !== this.filled.tab || !leaves(tab.split).includes(this.filled.session))) {
      this.filled = undefined;
    }
    const split: Split | undefined = tab
      ? this.filled
        ? { type: "leaf", session: this.filled.session }
        : tab.split
      : undefined;
    const key = tab && split ? `${tab.id} ${JSON.stringify(split)}` : "";
    if (key === this.shownKey) return;
    const shape = tab && split ? `${tab.id} ${shapeOf(split)}` : "";
    this.shownKey = key;
    // Only the sizes changed, after a divider drag: the boxes stay, so no terminal moves.
    if (split && this.layout && shape === this.shownShape) {
      this.applySizes(split, this.layout);
      return;
    }
    this.shownShape = shape;

    const shown = new Set<string>();
    const fresh: string[] = [];
    const layout = split ? this.build(split, shown, fresh, []) : undefined;
    this.layout = layout;
    for (const [session, entry] of this.entries) {
      if (!shown.has(session)) this.parked.append(entry.element);
    }
    if (layout) {
      this.root.replaceChildren(layout, this.parked);
    } else {
      this.root.replaceChildren(this.parked);
    }
    // Made once their boxes are in the page, so the first fit measures the real size.
    for (const session of fresh) {
      const entry = this.entries.get(session);
      if (entry) entry.view = this.make(entry.element, session);
    }
    this.updateDim();
  }

  /** Focuses the pane of the current tab that typing last went to, else its first pane. */
  focusCurrent(): void {
    const session = this.focusedSession();
    if (session !== undefined) this.entries.get(session)?.view?.focus();
  }

  focusSession(session: string): void {
    this.entries.get(session)?.view?.focus();
  }

  /** The current tab's focused pane. */
  focusedSession(): string | undefined {
    if (!this.tab) return undefined;
    if (this.filled) return this.filled.session;
    const sessions = leaves(this.tab.split);
    const last = this.focused.get(this.tab.id);
    return last !== undefined && sessions.includes(last) ? last : sessions[0];
  }

  /** Moves typing to the pane beside the focused one (Cmd+Option+arrows). */
  focusNeighbor(direction: PaneDirection): void {
    const from = this.focusedSession();
    if (!this.tab || this.filled || from === undefined) return;
    const box = (session: string) => this.entries.get(session)?.element.getBoundingClientRect();
    const start = box(from);
    if (!start) return;
    const others: [string, Box][] = [];
    for (const session of leaves(this.tab.split)) {
      const rect = session === from ? undefined : box(session);
      if (rect) others.push([session, rect]);
    }
    const next = pickNeighbor(start, others, direction);
    if (next !== undefined) this.focusSession(next);
  }

  /** One pane fills the tab, or the split comes back. Nothing is sent to the host. */
  toggleFill(): void {
    if (!this.tab) return;
    if (this.filled) {
      this.filled = undefined;
    } else {
      const session = this.focusedSession();
      if (session === undefined || leaves(this.tab.split).length < 2) return;
      this.filled = { tab: this.tab.id, session };
    }
    this.shownKey = "";
    this.show(this.tab);
    this.focusCurrent();
  }

  /** With this on, pointing at a pane gives it typing (R31). */
  setFocusFollowsMouse(on: boolean): void {
    this.followMouse = on;
  }

  /** The `dim` setting: panes without typing drop to 58 percent. Off by default (R32). */
  setDim(on: boolean): void {
    if (on === this.dim) return;
    this.dim = on;
    this.root.classList.toggle("dim-panes", on);
    if (on) {
      this.updateDim();
    } else {
      for (const view of this.all()) view.setDimmed(false);
    }
  }

  viewFor(session: string): TerminalView | undefined {
    return this.entries.get(session)?.view;
  }

  all(): TerminalView[] {
    return [...this.entries.values()].flatMap((entry) => (entry.view ? [entry.view] : []));
  }

  sessions(): string[] {
    return [...this.entries.keys()];
  }

  /** Ends the views of sessions that are no longer in any tab. */
  prune(alive: Set<string>): void {
    for (const [session, entry] of this.entries) {
      if (alive.has(session)) continue;
      entry.view?.dispose();
      entry.element.remove();
      this.entries.delete(session);
    }
  }

  // Never runs with the setting off, so the default costs nothing on a focus change.
  private updateDim(): void {
    if (!this.dim || !this.tab) return;
    const focused = this.focusedSession();
    for (const [session, entry] of this.entries) entry.view?.setDimmed(session !== focused);
  }

  private build(split: Split, shown: Set<string>, fresh: string[], path: number[]): HTMLElement {
    if (split.type === "leaf") {
      shown.add(split.session);
      const element = this.entry(split.session, fresh).element;
      // A pane that was part of a split keeps no stale share once it fills the tab alone.
      element.style.flexGrow = "";
      return element;
    }
    const box = document.createElement("div");
    box.className = `split split-${split.type}`;
    split.children.forEach((child, index) => {
      if (index > 0) {
        const divider = document.createElement("div");
        divider.className = "divider";
        divider.addEventListener("pointerdown", (event) => this.dragDivider(event, divider, path, index));
        box.append(divider);
      }
      const element = this.build(child, shown, fresh, [...path, index]);
      element.style.flexGrow = String(split.sizes[index] ?? 1);
      box.append(element);
    });
    return box;
  }

  private applySizes(split: Split, element: HTMLElement): void {
    if (split.type === "leaf") return;
    const kids = [...element.children].filter((child) => !child.classList.contains("divider")) as HTMLElement[];
    split.children.forEach((child, index) => {
      const kid = kids[index];
      if (!kid) return;
      kid.style.flexGrow = String(split.sizes[index] ?? 1);
      this.applySizes(child, kid);
    });
  }

  // Drags the divider between children `index - 1` and `index` of the box at `path`. Live while
  // dragging; on release the sizes are rounded to whole cells and sent as `set_sizes`.
  private dragDivider(event: PointerEvent, divider: HTMLElement, path: number[], index: number): void {
    const box = divider.parentElement;
    if (event.button !== 0 || !this.tab || !box) return;
    event.preventDefault();
    const tab = this.tab;
    const across = box.classList.contains("split-row");
    const kids = [...box.children].filter((child) => !child.classList.contains("divider")) as HTMLElement[];
    const sizes = kids.map((kid) => {
      const rect = kid.getBoundingClientRect();
      return across ? rect.width : rect.height;
    });
    const focused = this.focusedSession();
    const cellBox = (focused !== undefined ? this.viewFor(focused) : this.all()[0])?.cellSize();
    const cell = cellBox ? (across ? cellBox.width : cellBox.height) : 0;
    const least = Math.max(cell, 1);
    const start = across ? event.clientX : event.clientY;
    const a = index - 1;
    const b = index;
    const pair = sizes[a] + sizes[b];
    // In pixels from here on, so the two panes move exactly with the pointer.
    kids.forEach((kid, i) => {
      kid.style.flexGrow = String(sizes[i]);
    });
    divider.setPointerCapture(event.pointerId);
    const startA = sizes[a];
    const move = (moveEvent: PointerEvent) => {
      const delta = (across ? moveEvent.clientX : moveEvent.clientY) - start;
      sizes[a] = Math.min(pair - least, Math.max(least, startA + delta));
      sizes[b] = pair - sizes[a];
      kids[a].style.flexGrow = String(sizes[a]);
      kids[b].style.flexGrow = String(sizes[b]);
    };
    const up = () => {
      divider.removeEventListener("pointermove", move);
      divider.removeEventListener("pointerup", up);
      divider.removeEventListener("pointercancel", up);
      const rounded = roundToCells(sizes, cell);
      kids.forEach((kid, i) => {
        kid.style.flexGrow = String(rounded[i]);
      });
      if (this.tab?.id === tab.id) this.onSizes(tab.id, withSizesAt(tab.split, path, rounded));
      // The preventDefault above also stops the mouseup the focus rule listens for.
      this.focusCurrent();
    };
    divider.addEventListener("pointermove", move);
    divider.addEventListener("pointerup", up);
    divider.addEventListener("pointercancel", up);
  }

  private entry(session: string, fresh: string[]): Entry {
    let entry = this.entries.get(session);
    if (!entry) {
      const element = document.createElement("div");
      element.className = "pane";
      element.addEventListener("focusin", () => {
        if (!this.tab) return;
        this.focused.set(this.tab.id, session);
        this.updateDim();
        this.onFocus();
      });
      element.addEventListener("mouseenter", () => {
        if (!this.followMouse || session === this.focusedSession()) return;
        // A rename field keeps typing until Enter or Esc.
        if (document.activeElement instanceof HTMLInputElement) return;
        this.entries.get(session)?.view?.focus();
      });
      entry = { element, view: undefined };
      this.entries.set(session, entry);
      fresh.push(session);
    }
    return entry;
  }
}
