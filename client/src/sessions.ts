// What the client knows of the host's sessions without asking: each pane's session number and the
// folder its shell is in. Numbers start over in a new host, so each connection gets its own.

import type { SessionItem } from "./connection";

type Creating = { cwd: string; done: Promise<void>; settle(): void };

export class Sessions {
  private readonly numbers = new Map<string, number>();
  private readonly cwds = new Map<string, string>();
  // Creates sent and not answered yet. The tree push showing the pane can come first.
  private readonly waiting = new Map<string, Creating>();

  /** One item of a `list` answer. */
  learn(item: Pick<SessionItem, "uuid" | "session" | "cwd">): void {
    this.numbers.set(item.uuid, item.session);
    if (item.cwd) this.cwds.set(item.uuid, item.cwd);
  }

  number(uuid: string): number | undefined {
    return this.numbers.get(uuid);
  }

  /** The folder the pane's shell last reported or was started in, if known. */
  cwd(uuid: string): string | undefined {
    return this.cwds.get(uuid);
  }

  /** A shell reported a new folder. Returns its pane, if known. */
  moved(session: number, cwd: string): string | undefined {
    for (const [uuid, number] of this.numbers) {
      if (number !== session) continue;
      this.cwds.set(uuid, cwd);
      return uuid;
    }
    return undefined;
  }

  /** Call before sending `create`, so a view made before `created` waits for its number. */
  creating(uuid: string, cwd: string): void {
    let settle = (): void => {};
    const done = new Promise<void>((resolve) => {
      settle = resolve;
    });
    this.waiting.set(uuid, { cwd, done, settle });
  }

  /** The create for `uuid` was answered with `session`. */
  created(uuid: string, session: number): void {
    const creating = this.waiting.get(uuid);
    this.waiting.delete(uuid);
    this.numbers.set(uuid, session);
    if (!creating) return;
    // Where the shell starts, until it reports a folder of its own.
    this.cwds.set(uuid, creating.cwd);
    creating.settle();
  }

  /** The create for `uuid` failed or was never answered. */
  failed(uuid: string): void {
    const creating = this.waiting.get(uuid);
    this.waiting.delete(uuid);
    creating?.settle();
  }

  /**
   * `uuid`'s number once the answer to this client's create is in, or else as `ask` (a `list`)
   * finds it, for a pane this client did not create, such as a reopened tab's.
   */
  async numberOf(uuid: string, ask: () => Promise<void>): Promise<number | undefined> {
    await this.waiting.get(uuid)?.done;
    if (!this.numbers.has(uuid)) await ask();
    return this.numbers.get(uuid);
  }
}
