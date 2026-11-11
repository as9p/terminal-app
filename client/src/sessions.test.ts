import { afterEach, describe, expect, it, vi } from "vitest";

import type { SessionItem, Split, Tab, Tree } from "./connection";
import { type ActionContext, runAction } from "./keys";
import { Sessions } from "./sessions";
import { Store } from "./store";
import { type AttachWiring, type PaneNames, attachView, folderReported } from "./wiring";

const NEW = "0-0-0-0-new";
const NEW2 = "0-0-0-0-two";

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
 * A host behind a fake connection that counts what the client sends. Each `create` waits until the
 * test answers it, so the tree push can come before or after `created`.
 */
function host(listed: SessionItem[] = []) {
  const answers = new Map<string, { resolve(session: number): void; reject(error: Error): void }>();
  const conn = {
    treeOp: vi.fn(),
    kill: vi.fn(),
    list: vi.fn(async () => listed),
    create: vi.fn(
      (uuid: string) =>
        new Promise<number>((resolve, reject) => {
          answers.set(uuid, { resolve, reject });
        }),
    ),
    attach: vi.fn<(session: number, cols: number, rows: number) => Promise<void>>(async () => {}),
    resize: vi.fn(),
  };
  return {
    conn,
    created: (session: number, uuid = NEW) => answers.get(uuid)?.resolve(session),
    refused: (uuid = NEW) => answers.get(uuid)?.reject(new Error("spawn_failed")),
  };
}

type FakeConnection = ReturnType<typeof host>["conn"];

function context(store: Store, sessions: Sessions, conn: FakeConnection): ActionContext {
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

/** A pane's view; attaching sends `attach` on the connection, as TerminalView does. */
function view() {
  return {
    attach: vi.fn(async (conn: FakeConnection, session: number) => {
      await conn.attach(session, 80, 24);
    }),
  };
}

/** What main.ts hands `attachView`: one connection, its record, and views open until closed. */
function wiring(sessions: Sessions, conn: FakeConnection) {
  const closed = new Set<unknown>();
  const deps: AttachWiring<FakeConnection> = {
    connection: () => conn,
    sessions: () => sessions,
    refresh: async (next, known) => {
      for (const listed of await next.list()) known.learn(listed);
    },
    isCurrent: (_uuid, made) => !closed.has(made),
    note: vi.fn(),
  };
  return { deps, close: (made: unknown) => closed.add(made) };
}

function setup(cwd: string | null = "/code/a", hostListsNew = true) {
  const store = new Store();
  store.setTree(tree([tab("a", leaf("s-a"))]));
  store.select("a");
  const sessions = new Sessions();
  sessions.learn(item("s-a", 1, cwd));
  const fake = host(hostListsNew ? [item("s-a", 1, cwd), item(NEW, 5, null)] : [item("s-a", 1, cwd)]);
  const ctx = context(store, sessions, fake.conn);
  vi.spyOn(crypto, "randomUUID").mockReturnValue(NEW);
  // The host's tree push with the new tab in it.
  const push = () => store.setTree(tree([tab("a", leaf("s-a")), tab("n", leaf(NEW))]));
  return { store, sessions, ...fake, ctx, push, ...wiring(sessions, fake.conn) };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("new tab without extra round trips", () => {
  it("created_before_the_tree_push_attaches_once_with_no_list", async () => {
    const { sessions, conn, ctx, created, push, deps } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.create).toHaveBeenCalledWith(NEW, "shell", "/code/a", 80, 24, { type: "new_tab" });
    created(5);
    await vi.waitFor(() => expect(sessions.number(NEW)).toBe(5));
    push();
    const made = view();
    await attachView(deps, made, NEW);
    expect(made.attach).toHaveBeenCalledWith(conn, 5);
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledWith(5, 80, 24);
    expect(conn.list).not.toHaveBeenCalled();
  });

  it("the_tree_push_before_created_attaches_once_created_comes_with_no_list", async () => {
    const { conn, ctx, created, push, deps } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    push();
    const attaching = attachView(deps, view(), NEW);
    await Promise.resolve();
    expect(conn.attach).not.toHaveBeenCalled();
    created(5);
    await attaching;
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledWith(5, 80, 24);
    expect(conn.list).not.toHaveBeenCalled();
    expect(ctx.panes.focusSession).toHaveBeenCalledWith(NEW);
  });

  it("a_split_shares_the_path", async () => {
    const { conn, ctx, created, deps } = setup();
    runAction("splitDown", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    expect(conn.create).toHaveBeenCalledWith(NEW, "shell", "/code/a", 80, 24, {
      type: "split_of",
      session: "s-a",
      dir: "down",
    });
    const attaching = attachView(deps, view(), NEW);
    created(5);
    await attaching;
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

  it("a_refused_create_attaches_nothing_and_settles_the_waiting_view", async () => {
    // The host made no session, so its `list` does not have one either.
    const { sessions, conn, ctx, refused, push, deps } = setup("/code/a", false);
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    push();
    const attaching = attachView(deps, view(), NEW);
    refused();
    await attaching;
    await vi.waitFor(() => expect(ctx.ui.note).toHaveBeenCalled());
    expect(conn.list).toHaveBeenCalledTimes(1);
    expect(conn.attach).not.toHaveBeenCalled();
    expect(sessions.cwd(NEW)).toBeUndefined();
  });

  it("a_pane_closed_before_its_number_is_known_attaches_nothing_and_leaves_nothing_waiting", async () => {
    const { sessions, conn, ctx, created, push, deps, close } = setup();
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalled());
    push();
    const made = view();
    const attaching = attachView(deps, made, NEW);
    await Promise.resolve();
    close(made);
    created(5);
    await attaching;
    expect(conn.attach).not.toHaveBeenCalled();
    expect(conn.list).not.toHaveBeenCalled();
    // The create is answered, so a later ask does not wait on it.
    const ask = vi.fn(async () => {});
    await expect(sessions.numberOf(NEW, ask)).resolves.toBe(5);
    expect(ask).not.toHaveBeenCalled();
  });

  it("two_tabs_opened_back_to_back_each_attach_to_their_own_session", async () => {
    const { store, conn, ctx, created, deps } = setup();
    vi.spyOn(crypto, "randomUUID").mockReturnValueOnce(NEW).mockReturnValueOnce(NEW2);
    runAction("newTab", ctx);
    runAction("newTab", ctx);
    await vi.waitFor(() => expect(conn.create).toHaveBeenCalledTimes(2));
    store.setTree(tree([tab("a", leaf("s-a")), tab("n", leaf(NEW)), tab("t", leaf(NEW2))]));
    const first = view();
    const second = view();
    const attaching = Promise.all([attachView(deps, first, NEW), attachView(deps, second, NEW2)]);
    // The host answers the second create first.
    created(6, NEW2);
    created(5, NEW);
    await attaching;
    expect(first.attach).toHaveBeenCalledTimes(1);
    expect(first.attach).toHaveBeenCalledWith(conn, 5);
    expect(second.attach).toHaveBeenCalledTimes(1);
    expect(second.attach).toHaveBeenCalledWith(conn, 6);
    expect(conn.list).not.toHaveBeenCalled();
  });

  it("a_pane_this_client_did_not_create_is_found_with_one_list", async () => {
    const { conn, deps } = setup();
    // A reopened tab: the host made the session, so no `created` is coming.
    await attachView(deps, view(), NEW);
    expect(conn.list).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledTimes(1);
    expect(conn.attach).toHaveBeenCalledWith(5, 80, 24);
  });
});

describe("folder reports", () => {
  function names() {
    return {
      titles: new Map<string, string>(),
      folders: new Map([["s-a", "a"]]),
      retitle: vi.fn<PaneNames["retitle"]>(),
    } satisfies PaneNames;
  }

  it("a_new_folder_renames_an_untitled_pane_at_once", () => {
    const { sessions } = setup();
    const shown = names();
    folderReported(sessions, shown, 1, "/code/a/sub");
    expect(shown.folders.get("s-a")).toBe("sub");
    expect(shown.retitle).toHaveBeenCalledTimes(1);
    expect(sessions.cwd("s-a")).toBe("/code/a/sub");
  });

  it("the_same_folder_again_changes_nothing", () => {
    const { sessions } = setup();
    const shown = names();
    folderReported(sessions, shown, 1, "/code/a/sub");
    // A shell that reports its folder on every prompt.
    folderReported(sessions, shown, 1, "/code/a/sub");
    folderReported(sessions, shown, 1, "/code/a/sub/");
    expect(shown.retitle).toHaveBeenCalledTimes(1);
  });

  it("a_titled_pane_keeps_its_title_and_is_not_renamed", () => {
    const { sessions } = setup();
    const shown = names();
    shown.titles.set("s-a", "vim");
    folderReported(sessions, shown, 1, "/code/a/sub");
    expect(shown.retitle).not.toHaveBeenCalled();
    // Still recorded, as a `list` records it.
    expect(shown.folders.get("s-a")).toBe("sub");
  });

  it("a_session_this_client_does_not_know_changes_nothing", () => {
    const { sessions } = setup();
    const shown = names();
    folderReported(sessions, shown, 9, "/elsewhere");
    expect(shown.retitle).not.toHaveBeenCalled();
    expect(shown.folders.size).toBe(1);
  });
});
