"""Gives the test app more tabs before a speed run, for TA_SPEED_TABS. Standard library only.

Usage: add-tabs.py <port> <data folder> <tabs wanted> <idle folder>

Waits for the app's own first tab, then asks the test host for the rest, started in the idle
folder so the test's shell program can tell it should sit idle. The app picks the tab it shows
by itself and keeps it when a pushed tree adds tabs, so the first tab stays showing; marking it
used again only keeps the host's order of use pointing at it.
"""

import os
import sys
import time
import uuid

# No __pycache__ left in the repo by each speed run.
sys.dont_write_bytecode = True
sys.path.insert(0,os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "host-client"))
from host_client import ClientError, Connection, read_token  # noqa: E402

# The app starts its host and makes its first tab in a few seconds; a cold start can be slow.
WAIT_SECONDS = 20
ANSWER_SECONDS = 10
# The size the app asks for before it knows its window, so the host has nothing odd to do.
COLS, ROWS = 80, 24


def connect(port, token_file, deadline):
    while True:
        try:
            conn = Connection(port, read_token(token_file))
            conn.sock.settimeout(ANSWER_SECONDS)
            return conn
        except ClientError:
            if time.monotonic() > deadline:
                raise
            time.sleep(0.25)


def answer(conn, wanted):
    """The next frame of type `wanted`, skipping pushed trees, states and output."""
    while True:
        try:
            frame = conn.hear()
        except (EOFError, OSError) as error:
            raise ClientError(f"the host stopped answering: {error or 'closed'}")
        if not isinstance(frame, dict):
            continue
        if frame.get("type") == wanted:
            return frame
        if frame.get("type") == "error":
            raise ClientError(f"host error {frame.get('code')}: {frame.get('message')}")


def tab_ids(conn):
    conn.say({"type": "tree_get"})
    tree = answer(conn, "tree")["tree"]
    tabs = [tab for group in tree.get("groups", []) for tab in group.get("tabs", [])]
    tabs += tree.get("ungrouped", [])
    return [tab["id"] for tab in tabs]


def main():
    if len(sys.argv) != 5:
        raise ClientError("usage: add-tabs.py <port> <data folder> <tabs wanted> <idle folder>")
    port, data, wanted, idle = int(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4]
    deadline = time.monotonic() + WAIT_SECONDS
    conn = connect(port, os.path.join(data, "host-token"), deadline)
    # Asking before the app has its tab would make the app see tabs and make none of its own,
    # and then a tab of ours, running the idle program, could be the one it shows.
    ids = tab_ids(conn)
    while not ids:
        if time.monotonic() > deadline:
            raise ClientError("the app made no tab")
        time.sleep(0.25)
        ids = tab_ids(conn)
    first = ids[0]
    for _ in range(wanted - len(ids)):
        conn.say(
            {
                "type": "create",
                "uuid": str(uuid.uuid4()),
                "kind": "shell",
                "cwd": idle,
                "cols": COLS,
                "rows": ROWS,
                "place": {"type": "new_tab"},
            }
        )
        answer(conn, "created")
    conn.say({"type": "tree_op", "op": {"type": "used", "tab": first}})
    ids = tab_ids(conn)
    conn.close()
    if len(ids) != wanted or ids[0] != first:
        raise ClientError(f"wanted {wanted} tabs with {first} first, the host has {ids}")


if __name__ == "__main__":
    try:
        main()
    except ClientError as error:
        print(f"add-tabs: {error}", file=sys.stderr)
        sys.exit(1)
