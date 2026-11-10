// The terminal area. One TerminalView per session, made the first time its tab is shown and
// kept, hidden, while another tab is on screen. A tab's split becomes nested flex boxes.

import type { Split, Tab } from "./connection";
import { leaves } from "./store";
import type { TerminalView } from "./terminal";

/** Makes the view for `session` inside `container`, which is already in the page. */
export type MakeView = (container: HTMLElement, session: string) => TerminalView;

type Entry = { element: HTMLElement; view: TerminalView | undefined };

export class Panes {
  private readonly root: HTMLElement;
  private readonly make: MakeView;
  private readonly onFocus: () => void;
  private readonly entries = new Map<string, Entry>();
  // Hidden views wait here. display: none, so they neither draw nor take layout work.
  private readonly parked: HTMLElement;
  private tab: Tab | undefined;
  private shownKey = "";
  // The pane typing last went to, per tab, so coming back to a tab lands in the same pane.
  private readonly focused = new Map<string, string>();

  constructor(root: HTMLElement, make: MakeView, onFocus: () => void = () => {}) {
    this.root = root;
    this.make = make;
    this.onFocus = onFocus;
    this.parked = document.createElement("div");
    this.parked.className = "parked";
    this.root.append(this.parked);
  }

  /** Puts `tab` on screen. The same tab with the same split changes nothing in the page. */
  show(tab: Tab | undefined): void {
    this.tab = tab;
    const key = tab ? `${tab.id} ${JSON.stringify(tab.split)}` : "";
    if (key === this.shownKey) return;
    this.shownKey = key;

    const shown = new Set<string>();
    const fresh: string[] = [];
    const layout = tab ? this.build(tab.split, shown, fresh) : undefined;
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
    const sessions = leaves(this.tab.split);
    const last = this.focused.get(this.tab.id);
    return last !== undefined && sessions.includes(last) ? last : sessions[0];
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

  private build(split: Split, shown: Set<string>, fresh: string[]): HTMLElement {
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
        box.append(divider);
      }
      const element = this.build(child, shown, fresh);
      element.style.flexGrow = String(split.sizes[index] ?? 1);
      box.append(element);
    });
    return box;
  }

  private entry(session: string, fresh: string[]): Entry {
    let entry = this.entries.get(session);
    if (!entry) {
      const element = document.createElement("div");
      element.className = "pane";
      element.addEventListener("focusin", () => {
        if (!this.tab) return;
        this.focused.set(this.tab.id, session);
        this.onFocus();
      });
      entry = { element, view: undefined };
      this.entries.set(session, entry);
      fresh.push(session);
    }
    return entry;
  }
}
