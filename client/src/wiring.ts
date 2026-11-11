// Steps main.ts runs between the connection, the session record and the views, kept out of
// main() so the tests run this code rather than a copy of it.

import type { Sessions } from "./sessions";

/** What attaching needs of a view: TerminalView in the app. */
export type AttachableView<C> = { attach(connection: C, session: number): Promise<void> };

export type AttachWiring<C> = {
  /** The connection now; a reconnect replaces it. */
  connection(): C | undefined;
  /** The record for the connection now. */
  sessions(): Sessions;
  /** A `list`, putting what it finds into `known`. */
  refresh(next: C, known: Sessions): Promise<void>;
  /** Whether `view` is still the view of `uuid`'s pane. */
  isCurrent(uuid: string, view: AttachableView<C>): boolean;
  /** Shows why an attach failed. */
  note(error: unknown): void;
};

/** Attaches a pane's view once its session number is known, whether the view or `created` came first. */
export async function attachView<C>(
  wiring: AttachWiring<C>,
  view: AttachableView<C>,
  uuid: string,
): Promise<void> {
  const next = wiring.connection();
  if (!next) return;
  const known = wiring.sessions();
  try {
    let session = known.number(uuid);
    if (session === undefined) {
      // A pane this client created gets its number from `created`; only others need a `list`.
      session = await known.numberOf(uuid, () => wiring.refresh(next, known));
      // Its pane may have closed while it waited.
      if (!wiring.isCurrent(uuid, view)) return;
    }
    // Gone already: the tree push that drops its pane is on its way.
    if (session === undefined || wiring.connection() !== next) return;
    await view.attach(next, session);
  } catch (error) {
    if (wiring.connection() === next) wiring.note(error);
  }
}

export function folderName(path: string): string {
  const parts = path.split("/").filter((part) => part !== "");
  return parts[parts.length - 1] ?? path;
}

/** The sidebar's names for panes: a terminal title wins, else the folder. */
export type PaneNames = {
  readonly titles: ReadonlyMap<string, string>;
  readonly folders: Map<string, string>;
  /** Puts new names on the rows already in the sidebar. */
  retitle(): void;
};

/**
 * A shell reported its folder. An untitled pane is named for it, so its row is renamed now rather
 * than at the next sidebar render. Most reports repeat the last folder (a shell can report on every
 * prompt), and those stop at one lookup and one compare.
 */
export function folderReported(
  known: Pick<Sessions, "moved">,
  names: PaneNames,
  session: number,
  cwd: string,
): void {
  const uuid = known.moved(session, cwd);
  if (uuid === undefined) return;
  const name = folderName(cwd);
  if (names.folders.get(uuid) === name) return;
  names.folders.set(uuid, name);
  if (!names.titles.has(uuid)) names.retitle();
}
