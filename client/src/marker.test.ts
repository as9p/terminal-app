import { afterEach, describe, expect, it, vi } from "vitest";

import type { Group, Tab, Tree } from "./connection";
import { MORPH_RUN_MS, createMarker, markerState, setMarkerState } from "./marker";
import { Sidebar } from "./sidebar";
import { Store } from "./store";
import css from "./styles.css?raw";

vi.mock("./focus", () => ({ refocus: vi.fn() }));

function tab(id: string, name: string | null = null): Tab {
  return { id, name, pinned: false, split: { type: "leaf", session: `s-${id}` }, last_used: 0 };
}

function group(key: string, tabs: Tab[], collapsed = false): Group {
  return { key, path: `/code/${key}`, name: null, branch: "main", collapsed, tabs };
}

function tree(groups: Group[], ungrouped: Tab[] = []): Tree {
  return { groups, ungrouped, closed: [], defaults: {} };
}

function mount(t: Tree, titles: Record<string, string> = {}) {
  const element = document.createElement("aside");
  document.body.append(element);
  const store = new Store();
  store.setTree(t);
  const sidebar = new Sidebar(element, store, {
    select: (id) => store.select(id),
    collapse: vi.fn(),
    focusPane: vi.fn(),
    title: (session) => titles[session],
  });
  store.subscribe(() => sidebar.render());
  store.subscribeStates(() => sidebar.updateStates());
  sidebar.render();
  return { element, store, sidebar };
}

function rowMarker(element: HTMLElement, id: string): HTMLElement | null {
  return element.querySelector<HTMLElement>(`.row[data-tab="${id}"] > .slot > .mk`);
}

function reduceMotion(on: boolean): void {
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: on && query.includes("reduce"), media: query }));
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

describe("marker rig", () => {
  it("draws_every_state_from_one_rig_of_six_strokes", () => {
    const mk = createMarker("working");
    expect(mk.matches(".mk.a.mo.s-working")).toBe(true);
    expect(mk.getAttribute("role")).toBe("img");
    expect(mk.getAttribute("aria-label")).toBe("Working");
    expect(mk.title).toBe("Working");
    const svg = mk.querySelector(":scope > .fset > .fspin > svg.mrig");
    expect(svg?.classList.contains("p-working")).toBe(true);
    expect(svg?.getAttribute("viewBox")).toBe("-8 -8 16 16");
    expect(svg?.querySelectorAll(".ly")).toHaveLength(2);
    expect(svg?.querySelectorAll(".ly1 .sk")).toHaveLength(6);
    expect(svg?.querySelectorAll(".ly1 .fl")).toHaveLength(1);
    expect(markerState(mk)).toBe("working");
  });

  it("a_working_marker_starts_its_tick_from_the_clock", () => {
    vi.spyOn(performance, "now").mockReturnValue(9_250);
    const mk = createMarker("working");
    expect(mk.querySelector<HTMLElement>(".fspin")?.style.animationDelay).toBe("-1250ms");
    expect(createMarker("idle").querySelector<HTMLElement>(".fspin")?.style.animationDelay).toBe("");
  });

  it("a_state_change_morphs_the_same_marker_then_settles", () => {
    vi.useFakeTimers();
    reduceMotion(false);
    const mk = createMarker("working");
    document.body.append(mk);
    setMarkerState(mk, "blocked");
    const svg = mk.querySelector(".mrig");
    expect(mk.classList.contains("s-blocked")).toBe(true);
    expect(svg?.classList.contains("p-blocked")).toBe(true);
    expect(svg?.classList.contains("p-working")).toBe(false);
    expect(mk.classList.contains("mo-run")).toBe(true);
    expect(mk.classList.contains("mo-hold")).toBe(true);
    expect(mk.getAttribute("aria-label")).toBe("Blocked, waiting on you");
    vi.advanceTimersByTime(MORPH_RUN_MS);
    expect(mk.classList.contains("mo-run")).toBe(false);
    expect(mk.classList.contains("mo-hold")).toBe(false);
    expect(mk.querySelector<HTMLElement>(".fset")?.style.transform).toBe("");
  });

  it("reduced_motion_switches_at_once", () => {
    reduceMotion(true);
    const mk = createMarker("done");
    document.body.append(mk);
    setMarkerState(mk, "idle");
    expect(mk.querySelector(".mrig")?.classList.contains("p-idle")).toBe(true);
    expect(mk.classList.contains("mo-run")).toBe(false);
    expect(mk.classList.contains("mo-hold")).toBe(false);
  });

  it("styles_hold_all_six_poses_and_the_morph_timing", () => {
    for (const state of ["working", "blocked", "idle", "done", "failed", "handoff"]) {
      for (const piece of ["k0", "k1", "k2", "k3", "k4", "k5", "fl"]) {
        expect(css).toContain(`.mrig.p-${state} .${piece} {`);
      }
    }
    expect(css).toContain("--mo-ms: 320ms; --mo-step: 18ms; --mo-ease: cubic-bezier(0.34, 1.4, 0.64, 1);");
    expect(css).toContain("animation: fl-tick 4s linear infinite");
  });
});

describe("markers in the sidebar", () => {
  it("state_change_updates_the_marker_in_place", () => {
    reduceMotion(false);
    const { element, store } = mount(tree([], [tab("a"), tab("b")]));
    store.setStates([{ session: "s-b", state: "working", since_ms: 1 }]);
    const before = rowMarker(element, "b");
    const row = element.querySelector('.row[data-tab="b"]');
    expect(before?.classList.contains("s-working")).toBe(true);
    store.setStates([{ session: "s-b", state: "blocked", since_ms: 2 }]);
    expect(element.querySelector('.row[data-tab="b"]')).toBe(row);
    expect(rowMarker(element, "b")).toBe(before);
    expect(before?.querySelector(".mrig")?.classList.contains("p-blocked")).toBe(true);
    expect(before?.classList.contains("mo-run")).toBe(true);
    expect(rowMarker(element, "a")).toBeNull();
  });

  it("tree_rebuild_keeps_the_pose_without_a_morph", () => {
    reduceMotion(false);
    const { element, store } = mount(tree([], [tab("a"), tab("b")]));
    store.setStates([{ session: "s-b", state: "working", since_ms: 1 }]);
    store.setStates([{ session: "s-b", state: "done", since_ms: 2 }]);
    store.select("b");
    const after = rowMarker(element, "b");
    expect(after?.querySelector(".mrig")?.classList.contains("p-done")).toBe(true);
    expect(after?.classList.contains("mo-run")).toBe(false);
    expect(after?.classList.contains("mo-hold")).toBe(false);
  });

  it("collapsed_group_rolls_up_blocked_in_place", () => {
    const { element, store } = mount(tree([group("g", [tab("g1")], true)]));
    const head = element.querySelector(".ghead");
    expect(head?.querySelector(".roll")).toBeNull();
    store.setStates([{ session: "s-g1", state: "blocked", since_ms: 1 }]);
    expect(element.querySelector(".ghead")).toBe(head);
    const roll = head?.querySelector(":scope > .gcount + .roll > .mk");
    expect(roll?.querySelector(".mrig")?.classList.contains("p-blocked")).toBe(true);
    store.setStates([{ session: "s-g1", state: "working", since_ms: 2 }]);
    expect(head?.querySelector(".roll")).toBeNull();
  });

  it("blocked_and_done_rows_get_a_dim_line_with_the_title", () => {
    const { element, store } = mount(tree([], [tab("a", "api"), tab("b", "web")]), {
      "s-a": "Fix the login",
      "s-b": "Tests",
    });
    store.setStates([
      { session: "s-a", state: "blocked", since_ms: 1 },
      { session: "s-b", state: "working", since_ms: 1 },
    ]);
    expect(element.querySelector('.row[data-tab="a"] .sub')?.textContent).toBe("Fix the login");
    expect(element.querySelector('.row[data-tab="a"]')?.classList.contains("has-sub")).toBe(true);
    expect(element.querySelector('.row[data-tab="b"] .sub')).toBeNull();
    store.setStates([{ session: "s-a", state: "idle", since_ms: 2 }]);
    expect(element.querySelector('.row[data-tab="a"] .sub')).toBeNull();
  });
});
