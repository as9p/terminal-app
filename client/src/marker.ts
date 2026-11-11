// State markers (set a, docs/ui-review.md "State markers" and "Marker morph"). Every state is a
// pose of one rig of six strokes; a new pose class makes each stroke travel to its new place, so
// any marker reshapes into any other. Transform and opacity only; the poses are in styles.css.
// Ported from the review mock (mock 520-630, 810-845, 1618-1690).

import type { AgentState } from "./connection";

/** The four states the host sends, plus two looks recorded for a later plan. Nothing sets those yet. */
export type MarkerState = AgentState | "failed" | "handoff";

/** A marker and the state it is to show. */
export type MarkerChange = readonly [HTMLElement, MarkerState];

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
// The first quarter of `fl-tick` in styles.css, as [percent of the turn, degrees]. The other three
// quarters repeat it 15, 30 and 45 degrees on.
const TICK_Q1: readonly (readonly [number, number])[] = [
  [0, 0], [1, 1.86], [2, 5.73], [3, 9.77], [4, 13], [5, 15.09], [6.25, 16.27], [7.5, 16.37],
  [9, 15.88], [10.75, 15.26], [12.75, 14.91], [15, 14.88], [17.5, 15], [25, 15],
];

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
// The batch whose timer settles each morphing marker. A later change takes the marker over.
const owner = new WeakMap<HTMLElement, object>();

function reducedMotion(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

const modTo = (x: number, n: number): number => ((x % n) + n) % n;

/**
 * Where in the 4 s tick a flake starts at `now` (ms on the page clock, the one time origin every
 * flake shares), so flakes started at different moments turn in step.
 */
export function tickPhase(now: number): number {
  return modTo(Math.round(now), TICK_MS);
}

/** The turn in degrees of a ticking flake `phase` ms into the tick, read off the keyframes. */
export function tickTurn(phase: number): number {
  const pct = (modTo(phase, TICK_MS) / TICK_MS) * 100;
  const quarter = Math.floor(pct / 25);
  const p = pct - quarter * 25;
  for (let i = 1; i < TICK_Q1.length; i++) {
    const [p1, d1] = TICK_Q1[i];
    if (p > p1) continue;
    const [p0, d0] = TICK_Q1[i - 1];
    return quarter * 15 + d0 + ((d1 - d0) * (p - p0)) / (p1 - p0);
  }
  return quarter * 15 + 15;
}

/** Starts a working flake's tick where the clock is in the cycle. Returns that phase in ms. */
export function syncTick(mk: HTMLElement, now = performance.now()): number {
  const phase = tickPhase(now);
  const spin = mk.querySelector<HTMLElement>(".fspin");
  if (spin) spin.style.animationDelay = `-${phase}ms`;
  return phase;
}

/**
 * Puts every ticking flake under `root` back in step with the clock, after the window comes back
 * and the paused ticks resume from wherever they stopped. One style pass for all of them; the
 * tick restarts because its animation is taken off and put back with a new start. A flake still
 * morphing into work is left alone: its turn on .fset was worked out against the tick it has, so
 * a restart would jump. Its settle puts it in step.
 */
export function resyncTicks(root: ParentNode): void {
  const spins = [...root.querySelectorAll<HTMLElement>(".mk.s-working:not(.mo-hold) .fspin")];
  if (spins.length === 0) return;
  const delay = `-${tickPhase(performance.now())}ms`;
  for (const spin of spins) spin.style.animationName = "none";
  void getComputedStyle(spins[0]).animationName;
  for (const spin of spins) {
    spin.style.removeProperty("animation-name");
    spin.style.animationDelay = delay;
  }
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
const turnOf = (el: Element): number => {
  const v = matrixOf(getComputedStyle(el).transform);
  const turn = (Math.atan2(v[1], v[0]) * 180) / Math.PI;
  return Number.isFinite(turn) ? turn : 0;
};
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

type Move = {
  mk: HTMLElement;
  was: MarkerState;
  next: MarkerState;
  svg: Element;
  fset: HTMLElement;
  ly0: HTMLElement;
  // Unset for a marker that changes at once.
  seen?: {
    running: boolean;
    color: string;
    color0: string;
    top: number;
    turn: number;
    partEls: SVGElement[];
    parts: (readonly [string, string])[];
  };
};

/** Turns the same marker element into `next`. See `setMarkerStates`. */
export function setMarkerState(mk: HTMLElement, next: MarkerState): void {
  setMarkerStates([[mk, next]]);
}

/**
 * Turns each marker element into its new state, in place. Off the page or under reduced motion a
 * marker changes at once; otherwise every stroke, the fill, the color and the turn travel to the
 * new pose, and a change in the middle of a morph starts from where every stroke had got to.
 *
 * All the markers go together so the page is styled and laid out once, not once per marker: first
 * every read of where each marker is, then every write, one forced reflow so the snapped look is
 * where the transitions start, and the rest of the writes. One timer settles the whole batch.
 */
export function setMarkerStates(changes: readonly MarkerChange[]): void {
  const reduce = reducedMotion();
  const moves: Move[] = [];

  // Read: where every old marker is (mock morphSnap), before any marker is touched.
  for (const [mk, next] of changes) {
    const was = markerState(mk);
    const svg = mk.querySelector(".mrig");
    const fset = mk.querySelector<HTMLElement>(".fset");
    const spin = mk.querySelector<HTMLElement>(".fspin");
    const ly0 = mk.querySelector<HTMLElement>(".ly0");
    const ly1 = mk.querySelector<HTMLElement>(".ly1");
    if (was === null || was === next || !svg || !fset || !spin || !ly0 || !ly1) continue;
    if (!mk.isConnected || reduce) {
      moves.push({ mk, was, next, svg, fset, ly0 });
      continue;
    }
    const running = mk.classList.contains("mo-run");
    const upper = getComputedStyle(ly1);
    // Only a morph in flight leaves a turn on .fset, and only a working flake ticks on .fspin.
    let turn = running ? turnOf(fset) : 0;
    if (was === "working" && !mk.classList.contains("off")) turn += turnOf(spin);
    const partEls = running ? [...mk.querySelectorAll<SVGElement>(MO_PARTS)] : [];
    const parts = partEls.map((el) => {
      const style = getComputedStyle(el);
      return [style.transform, style.opacity] as const;
    });
    moves.push({
      mk,
      was,
      next,
      svg,
      fset,
      ly0,
      seen: {
        running,
        color: upper.color,
        color0: getComputedStyle(ly0).color,
        top: parseFloat(upper.opacity),
        turn,
        partEls,
        parts,
      },
    });
  }
  if (moves.length === 0) return;

  // Write: each marker back to where the old one was, with no motion.
  const now = performance.now();
  const batch = {};
  const morphs: (Move & { end: number })[] = [];
  for (const move of moves) {
    const { mk, was, next, svg, fset, ly0, seen } = move;
    relabel(mk, was, next);
    const phase = next === "working" ? syncTick(mk, now) : 0;
    if (!seen) {
      owner.delete(mk);
      settle(mk);
      setPose(svg, was, next);
      continue;
    }
    owner.set(mk, batch);
    // The flake's turn moves to .fset and eases to rest: within 30 degrees when the flake was
    // still (any 60 degree step of the flake looks the same), else from the nearest equal turn.
    // A flake that now ticks is already at its tick's turn, so .fset takes the difference.
    const still = was === "working" && !seen.running;
    const spinTurn = next === "working" && !mk.classList.contains("off") ? tickTurn(phase) : 0;
    const start = nearTurn(seen.turn, still ? 60 : 360) - spinTurn;
    const end = next === "working" ? Math.round(start / 60) * 60 : 0;
    mk.classList.add("mo-snap", "mo-from", "mo-hold");
    ly0.style.color = seen.top >= 0.5 ? seen.color : seen.color0;
    seen.partEls.forEach((el, i) => {
      el.style.transform = seen.parts[i][0];
      el.style.opacity = seen.parts[i][1];
    });
    fset.style.transform = `rotate(${start}deg)`;
    morphs.push({ ...move, end });
  }
  if (morphs.length === 0) return;
  void morphs[0].mk.getBoundingClientRect();

  // Then let go: every stroke, the fill, the new color and the turn travel to the new pose.
  for (const { mk, was, next, svg, fset, seen, end } of morphs) {
    mk.classList.remove("mo-snap", "mo-from");
    mk.classList.add("mo-run");
    setPose(svg, was, next);
    for (const el of seen?.partEls ?? []) {
      el.style.removeProperty("transform");
      el.style.removeProperty("opacity");
    }
    fset.style.transform = `rotate(${end}deg)`;
  }
  window.setTimeout(() => settleBatch(batch, morphs), MORPH_RUN_MS);
}

// Hands every marker of a finished batch back to the plain marker, together: the bottom copy
// hides and the turn left on .fset clears, with one reflow for the batch. A marker a later change
// took over is left to that change. A flake that now ticks restarts its tick in step with the
// clock in the same reflow, since a window focus during the morph left it out of the resync.
function settleBatch(batch: object, morphs: readonly Move[]): void {
  const live: Move[] = [];
  for (const move of morphs) {
    if (owner.get(move.mk) !== batch) continue;
    owner.delete(move.mk);
    if (move.mk.isConnected) live.push(move);
    else settle(move.mk);
  }
  if (live.length === 0) return;
  const spins: HTMLElement[] = [];
  for (const { mk, next, fset, ly0 } of live) {
    mk.classList.add("mo-snap");
    mk.classList.remove("mo-run", "mo-hold");
    ly0.style.removeProperty("color");
    fset.style.removeProperty("transform");
    const spin = next === "working" ? mk.querySelector<HTMLElement>(".fspin") : null;
    if (spin) {
      spin.style.animationName = "none";
      spins.push(spin);
    }
  }
  void live[0].mk.getBoundingClientRect();
  for (const { mk } of live) mk.classList.remove("mo-snap");
  if (spins.length === 0) return;
  const delay = `-${tickPhase(performance.now())}ms`;
  for (const spin of spins) {
    spin.style.removeProperty("animation-name");
    spin.style.animationDelay = delay;
  }
}

/**
 * Puts `state` in `slot`: the marker there changes in place, one is made when there was none,
 * and it goes when the state is null. With `later`, a change to the marker there is added to it
 * for the caller to make with others in one `setMarkerStates`. Returns the marker shown.
 */
export function showMarker(
  slot: HTMLElement,
  state: MarkerState | null,
  later?: MarkerChange[],
): HTMLElement | null {
  const mk = slot.querySelector<HTMLElement>(":scope > .mk");
  if (state === null) {
    mk?.remove();
    return null;
  }
  if (mk) {
    if (!later) setMarkerState(mk, state);
    else if (markerState(mk) !== state) later.push([mk, state]);
    return mk;
  }
  const made = createMarker(state);
  slot.append(made);
  return made;
}
