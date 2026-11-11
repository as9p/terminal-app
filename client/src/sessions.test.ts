import { afterEach, describe, expect, it, vi } from "vitest";

import type { SessionItem, Split, Tab, Tree } from "./connection";
import { type ActionContext, runAction } from "./keys";
import { Sessions } from "./sessions";
import { Store } from "./store";

const NEW = "0-0-0-0-new";

function leaf(session: string): Split {
  return { type: "leaf", session };
}

function tab(id: string, split: Split): Tab {
  return { id, name: null, pinned: false, split, last_used: 0 };
}

function tree(tabs: Tab[]): Tree {
  return { groups: [], ungrouped: tabs, closed: [], defaults: {} };
}

function item(uuid: string, session: number, cwd: string | null): SessionItem {
  return { session, uuid, kind: "shell", title: null, cwd, exited: null, cols: 80, rows: 24 };
}

/**
 * A host behind a fake connection that counts what the client sends. `create` waits until the
 * test answers it, so the tree push can come before or after `created`.
 */
function host(listed: SessionItem[] = []) {
  let answer: { resolve(session: number): void; reject(error: Error): void } | undefined;
  const conn = {
    treeOp: vi.fn(),
    kill: vi.fn(),
    list: vi.fn(async () => listed),
    create: vi.fn(
      () =>
        new Promise<number>((resolve, reject) => {
          answer = { resolve, reject };
        }),
    ),
    attach: vi.fn<(session: number, cols: number, rows: number) => Promise<void>>(async () => {}),
    resize: vi.fn(),
  };
  return {
    conn,
    created: (session: number) => answer?.resolve(session),
    refused: () => answer?.reject(new Error("spawn_failed")),
  };
}

function context(store: Store, sessions: Sessions, conn: ReturnType<typeof host>["conn"]): ActionContext {
  return {
    store,
    sessions,
    panes: {
      focusedSession: () => "s-a",
      focusSession: vi.fn(),
      viewFor: () => undefined,
      focusNeighbor: vi.fn(),
      toggleFill: vi.fn(),
      leaveFill: vi.fn(),
    },
    connection: conn,
    home: "/home",
    ui: { toggleSidebar: vi.fn(), rename: vi.fn(), zoom: vi.fn(), note: vi.fn() },
  };
}

/** What main.ts does when the tree push makes the new pane's view. */
async function viewMade(sessions: Sessions, conn: ReturnType<typeof host>["conn"], uuid: string): Promise<void> {
  const ask = async () => {
    for (const listed of await conn.list()) sessions.learn(listed);
  };
  const session = sessions.number(uuid) ?? (await sessions.numberOf(uuid, ask));
  if (session !== undefined) await conn.attach(session, 80, 24);
}

function setup(cwd: string | null = "/code/a") {
  const store = new Store();
  store.setTree(tree([tab("a", leaf("s-a"))]));
  store.select("a");
  const sessions = new Sessions();
  sessions.learn(item("s-a", 1, cwd));
  const fake = host([item("s-a", 1, cwd), item(NEW, 5, null)]);
  const ctx = context(store, sessions, fake.conn);
  vi.spyOn(crypto, "randomUUID").mockReturnValue(NEW);
  // The host's tree push with the new tab in it.
  const push = () => store.setTree(tree([tab("a", leaf("s-a")), tab("n", leaf(NEW))]));
  return { store, sessions, ...fake, ctx, push };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("new tab without extra round trips", () => {
  it("created_before_the_tree_push_attaches_once_with_no_list", async () => {
    const { sessions, conn, ctx, created, push } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.create).toHaveBeenCalledWith(NEW, "shell", "/code/a", 80, 24, { type: "new_tab" });
    created(5);
    await vi.waitFor(() => expect(sessions.number(NEW)).toBe(5));
    push();
    await viewMade(sessions, conn, NEW);
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledWith(5, 80, 24);
    expect(conn.list).not.toHaveBeenCalled();
  });

  it("the_tree_push_before_created_attaches_once_created_comes_with_no_list", async () => {
    const { sessions, conn, ctx, created, push } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    push();
    const made = viewMade(sessions, conn, NEW);
    await Promise.resolve();
    expect(conn.attach).not.toHaveBeenCalled();
    created(5);
    await made;
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledWith(5, 80, 24);
    expect(conn.list).not.toHaveBeenCalled();
    expect(ctx.panes.focusSession).toHaveBeenCalledWith(NEW);
  });

  it("a_split_shares_the_path", async () => {
    const { sessions, conn, ctx, created } = setup();
    runAction("splitDown", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.create).toHaveBeenCalledWith(NEW, "shell", "/code/a", 80, 24, {
      type: "split_of",
      session: "s-a",
      dir: "down",
    });
    const made = viewMade(sessions, conn, NEW);
    created(5);
    await made;
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.list).not.toHaveBeenCalled();
  });

  it("a_reported_folder_is_where_the_next_shell_opens", async () => {
    const { sessions, conn, ctx } = setup();
    expect(sessions.moved(1, "/code/a/sub")).toBe("s-a");
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.create.mock.calls[0]).toContain("/code/a/sub");
    expect(conn.list).not.toHaveBeenCalled();
  });

  it("a_new_shell_is_known_in_the_folder_it_was_created_in", async () => {
    const { sessions, conn, ctx, created } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    created(5);
    await vi.waitFor(() => expect(sessions.cwd(NEW)).toBe("/code/a"));
  });

  it("a_pane_with_no_known_folder_asks_the_host_as_before", async () => {
    const { conn, ctx } = setup(null);
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.list).toHaveBeenCalledTimes(1);
    // The host has no folder either, so it opens at home as before.
    expect(conn.create.mock.calls[0]).toContain("/home");
  });

  it("no_focused_pane_opens_at_home_without_asking", async () => {
    const { conn, ctx } = setup();
    ctx.panes.focusedSession = () => undefined;
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.create.mock.calls[0]).toContain("/home");
    expect(conn.list).not.toHaveBeenCalled();
  });

  it("a_refused_create_lets_a_waiting_view_ask_the_host", async () => {
    const { sessions, conn, ctx, refused, push } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    push();
    const made = viewMade(sessions, conn, NEW);
    refused();
    await made;
    await vi.waitFor(() => expect(ctx.ui.note).toHaveBeenCalled());
    expect(conn.list).toHaveBeenCalledTimes(1);
    expect(sessions.cwd(NEW)).toBeUndefined();
  });

  it("a_pane_this_client_did_not_create_is_found_with_one_list", async () => {
    const { sessions, conn } = setup();
    // A reopened tab: the host made the session, so no `created` is coming.
    await viewMade(sessions, conn, NEW);
    expect(conn.list).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledWith(5, 80, 24);
  });
});
