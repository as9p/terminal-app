// Left Option acts as Alt while right Option still types special characters. xterm has one
// setting for both, so it follows the left key. One set of window listeners serves every view,
// so the work on each key press stays the same however many tabs are open.

type Apply = (meta: boolean) => void;

const views = new Set<Apply>();

function applyAll(meta: boolean): void {
  for (const apply of views) apply(meta);
}

const onKeyDown = (event: KeyboardEvent): void => {
  if (event.code === "AltLeft") applyAll(true);
};

const onKeyUp = (event: KeyboardEvent): void => {
  if (event.code === "AltLeft") applyAll(false);
};

const onBlur = (): void => applyAll(false);

/** Calls `apply` with true while left Option is held and false once it is let go. Returns the way to stop. */
export function followLeftOption(apply: Apply): () => void {
  if (views.size === 0) {
    // Capture, so xterm sees the new setting on the same key press that follows.
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);
  }
  views.add(apply);
  return () => {
    if (!views.delete(apply) || views.size > 0) return;
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("keyup", onKeyUp, true);
    window.removeEventListener("blur", onBlur);
  };
}
