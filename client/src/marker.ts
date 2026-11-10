// State markers (set a, docs/ui-review.md "State markers" and "Marker morph"). Every state is a
// pose of one rig of six strokes; a new pose class makes each stroke travel to its new place, so
// any marker reshapes into any other. Transform and opacity only; the poses are in styles.css.
// Ported from the review mock (mock 520-630, 810-845, 1618-1690).

import type { AgentState } from "./connection";

/** The four states the host sends, plus two looks recorded for a later plan. Nothing sets those yet. */
export type MarkerState = AgentState | "failed" | "handoff";

const LABEL: Record<MarkerState, string> = {
  working: "Working",
  blocked: "Blocked, waiting on you",
  done: "Done, not looked at",
  idle: "Idle",
  handoff: "Handoff requested, restart it from its handoff file",
  failed: "Failed, restart it or read the error",
};

const STATES = Object.keys(LABEL) as MarkerState[];

// The same numbers as --mo-ms and --mo-step in styles.css.
const MO_MS = 320;
const MO_STEP = 18;
/** How long a morph keeps its classes: the last stroke starts 90 ms in, plus some slack. */
export const MORPH_RUN_MS = MO_MS + 5 * MO_STEP + 80;
// One turn of the working tick, the same as `fl-tick 4s` in styles.css.
const TICK_MS = 4000;

// Each stroke is two 4.6 unit halves that slide apart to make it longer, plus a flake twig; the
// filled rounded square under them is the blocked and idle diamond. Two copies, so a color change
// is the new color fading in over the old one.
const RIG_LAYER =
  '<rect class="fl" x="-4.308" y="-4.308" width="8.615" height="8.615" rx="2.462" fill="currentColor" stroke="none"/>' +
  [0, 1, 2, 3, 4, 5]
    .map(
      (k) =>
        `<g class="sk k${k}" style="--i:${k}"><line class="sa" x1="-2.3" x2="2.3"/><line class="sb" x1="-2.3" x2="2.3"/><path class="tw" d="M0 0L1.8-1.9M0 0L1.8 1.9"/></g>`,
    )
    .join("");

const rigSvg = (state: MarkerState): string =>
  `<svg class="mrig p-${state}" width="13" height="13" viewBox="-8 -8 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><g class="ly ly0">${RIG_LAYER}</g><g class="ly ly1">${RIG_LAYER}</g></svg>`;

const MO_PARTS = ".sk, .sa, .sb, .tw, .fl";
const timers = new WeakMap<HTMLElement, number>();

function reducedMotion(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Starts a working flake's tick where the clock is in the cycle, so every flake turns in step. */
export function syncTick(mk: HTMLElement): void {
  const spin = mk.querySelector<HTMLElement>(".fspin");
  if (spin) spin.style.animationDelay = `-${Math.round(performance.now() % TICK_MS)}ms`;
}

/** A marker drawn at rest in `state`, with no morph. */
export function createMarker(state: MarkerState): HTMLElement {
  const mk = document.createElement("i");
  mk.className = `mk a mo s-${state}`;
  mk.setAttribute("role", "img");
  mk.setAttribute("aria-label", LABEL[state]);
  mk.title = LABEL[state];
  mk.innerHTML = `<span class="fset"><span class="fspin">${rigSvg(state)}</span></span>`;
  if (state === "working") syncTick(mk);
  return mk;
}

/** The state a marker shows. */
export function markerState(mk: Element): MarkerState | null {
  return STATES.find((state) => mk.classList.contains(`s-${state}`)) ?? null;
}

const matrixOf = (t: string): number[] =>
  !t || t === "none" ? [1, 0] : t.slice(t.indexOf("(") + 1).split(",").map(parseFloat);
const turnOf = (el: Element | null): number => {
  const v = matrixOf(el ? getComputedStyle(el).transform : "none");
  const turn = (Math.atan2(v[1], v[0]) * 180) / Math.PI;
  return Number.isFinite(turn) ? turn : 0;
};
const modTo = (x: number, n: number): number => ((x % n) + n) % n;
// The same look, turned as little as possible.
const nearTurn = (deg: number, step: number): number => modTo(deg + step / 2, step) - step / 2;

function relabel(mk: HTMLElement, from: MarkerState, to: MarkerState): void {
  mk.classList.remove(`s-${from}`);
  mk.classList.add(`s-${to}`);
  mk.setAttribute("aria-label", LABEL[to]);
  mk.title = LABEL[to];
}

function setPose(svg: Element, from: MarkerState, to: MarkerState): void {
  svg.classList.remove(`p-${from}`);
  svg.classList.add(`p-${to}`);
}

/** Leaves a marker plain: no morph classes, nothing set inline but the tick's start. */
function settle(mk: HTMLElement): void {
  mk.classList.remove("mo-run", "mo-hold", "mo-from", "mo-snap");
  mk.querySelector<HTMLElement>(".ly0")?.style.removeProperty("color");
  mk.querySelector<HTMLElement>(".fset")?.style.removeProperty("transform");
  for (const el of mk.querySelectorAll<SVGElement>(MO_PARTS)) {
    el.style.removeProperty("transform");
    el.style.removeProperty("opacity");
  }
}

/**
 * Turns the same marker element into `next`. Off the page or under reduced motion it changes at
 * once; otherwise every stroke, the fill, the color and the turn travel to the new pose. A change
 * in the middle of a morph starts from where every stroke had got to.
 */
export function setMarkerState(mk: HTMLElement, next: MarkerState): void {
  const was = markerState(mk);
  const svg = mk.querySelector(".mrig");
  const fset = mk.querySelector<HTMLElement>(".fset");
  const ly0 = mk.querySelector<HTMLElement>(".ly0");
  const ly1 = mk.querySelector<HTMLElement>(".ly1");
  if (was === null || was === next || !svg || !fset || !ly0 || !ly1) return;
  clearTimeout(timers.get(mk));
  timers.delete(mk);

  if (!mk.isConnected || reducedMotion()) {
    settle(mk);
    relabel(mk, was, next);
    setPose(svg, was, next);
    if (next === "working") syncTick(mk);
    return;
  }

  // Before: where the old marker is (mock morphSnap).
  const running = mk.classList.contains("mo-run");
  const color = getComputedStyle(ly1).color;
  const color0 = getComputedStyle(ly0).color;
  const top = parseFloat(getComputedStyle(ly1).opacity);
  const turn = turnOf(fset) + turnOf(mk.querySelector(".fspin"));
  const partEls = running ? [...mk.querySelectorAll<SVGElement>(MO_PARTS)] : [];
  const parts = partEls.map((el) => {
    const style = getComputedStyle(el);
    return [style.transform, style.opacity] as const;
  });

  relabel(mk, was, next);
  if (next === "working") syncTick(mk);
  // The flake's turn moves to .fset and eases to rest: within 30 degrees when the flake was still
  // (any 60 degree step of the flake looks the same), else from the nearest equal turn.
  const still = was === "working" && !running;
  const start = nearTurn(turn, still ? 60 : 360) - turnOf(mk.querySelector(".fspin"));
  const end = next === "working" ? Math.round(start / 60) * 60 : 0;

  // Back to where the old marker was, with no motion.
  mk.classList.add("mo-snap", "mo-from", "mo-hold");
  ly0.style.color = top >= 0.5 ? color : color0;
  partEls.forEach((el, i) => {
    el.style.transform = parts[i][0];
    el.style.opacity = parts[i][1];
  });
  fset.style.transform = `rotate(${start}deg)`;
  void mk.getBoundingClientRect();
  // Then let go: every stroke, the fill, the new color and the turn travel to the new pose.
  mk.classList.remove("mo-snap", "mo-from");
  mk.classList.add("mo-run");
  setPose(svg, was, next);
  for (const el of partEls) {
    el.style.removeProperty("transform");
    el.style.removeProperty("opacity");
  }
  fset.style.transform = `rotate(${end}deg)`;
  timers.set(
    mk,
    window.setTimeout(() => {
      timers.delete(mk);
      if (!mk.isConnected) {
        settle(mk);
        return;
      }
      // Hand back to the plain marker: the bottom copy hides, the turn left on .fset clears.
      mk.classList.add("mo-snap");
      mk.classList.remove("mo-run", "mo-hold");
      ly0.style.removeProperty("color");
      fset.style.removeProperty("transform");
      void mk.getBoundingClientRect();
      mk.classList.remove("mo-snap");
    }, MORPH_RUN_MS),
  );
}

/**
 * Puts `state` in `slot`: the marker there changes in place, one is made when there was none,
 * and it goes when the state is null. Returns the marker shown.
 */
export function showMarker(slot: HTMLElement, state: MarkerState | null): HTMLElement | null {
  const mk = slot.querySelector<HTMLElement>(":scope > .mk");
  if (state === null) {
    mk?.remove();
    return null;
  }
  if (mk) {
    setMarkerState(mk, state);
    return mk;
  }
  const made = createMarker(state);
  slot.append(made);
  return made;
}
