import "@xterm/xterm/css/xterm.css";
import "./fonts.css";

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { Connection, ConnectionFailure, HostError, type HostAddress } from "./connection";
import { TerminalView, terminalFontReady } from "./terminal";
import { theme } from "./theme";

type HostConnection = HostAddress & { home: string };

const RETRY_MS = 1000;
const RETRY_MAX_MS = 30_000;
const NOTE_MS = 4000;
const LOST_TEXT = "Lost the host, reconnecting";

/** Plain text over the terminal. A note lasts a moment, then whatever was there returns. */
class StatusBar {
  private readonly element: HTMLElement;
  private lasting = "";
  private noteTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(element: HTMLElement) {
    this.element = element;
  }

  show(text: string): void {
    this.lasting = text;
    if (this.noteTimer === undefined) this.render(text);
  }

  clear(): void {
    this.show("");
  }

  note(text: string): void {
    clearTimeout(this.noteTimer);
    this.render(text);
    this.noteTimer = setTimeout(() => {
      this.noteTimer = undefined;
      this.render(this.lasting);
    }, NOTE_MS);
  }

  private render(text: string): void {
    this.element.textContent = text;
    this.element.hidden = text === "";
  }
}

function inTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

function findHost(): Promise<HostConnection> {
  if (import.meta.env.DEV && !inTauri()) {
    const query = new URLSearchParams(location.search);
    return Promise.resolve({
      port: Number(query.get("port")),
      token: query.get("token") ?? "",
      home: query.get("home") ?? "",
    });
  }
  return invoke<HostConnection>("host_connection");
}

// A native write, because WKWebView can refuse navigator.clipboard outside a user gesture.
function copy(text: string): Promise<void> {
  if (import.meta.env.DEV && !inTauri()) return navigator.clipboard.writeText(text);
  return writeText(text);
}

function describe(error: unknown, port: number | undefined): string {
  if (error instanceof ConnectionFailure) {
    switch (error.kind) {
      case "proto_mismatch":
        return "The running host is a different version. Nothing was changed. Quit the host to update it.";
      case "token_refused":
        return `The host on port ${port} refused this app's token.`;
      case "not_our_host":
        return `Something answers on port ${port}, but it is not this app's host.`;
      case "lost":
        return LOST_TEXT;
    }
  }
  if (error instanceof HostError) return `The host says: ${error.message}`;
  return String(error);
}

const LAST_SESSION_KEY = "lastSession";

/** The uuid of the session the window last showed. Lost if storage is turned off. */
function lastSession(): string | undefined {
  try {
    return localStorage.getItem(LAST_SESSION_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

function rememberSession(uuid: string): void {
  try {
    localStorage.setItem(LAST_SESSION_KEY, uuid);
  } catch {
    // Full or turned off: the next window starts on the first live session instead.
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function main(): Promise<void> {
  const barElement = document.querySelector<HTMLElement>("#bar");
  const parent = document.querySelector<HTMLElement>("#terminal");
  if (!barElement || !parent) return;
  barElement.style.background = theme.black ?? "";
  barElement.style.color = theme.foreground ?? "";
  const bar = new StatusBar(barElement);

  await terminalFontReady();

  let connection: Connection | undefined;
  let sessionUuid: string | undefined;
  let bringingUp = false;
  // After a crash the screen is what matters, and nothing on it needs the host any more.
  let stopped = false;

  const view = new TerminalView(parent, {
    onSelection: (text) => {
      copy(text).catch((error: unknown) => bar.note(`Could not copy the selection: ${error}`));
    },
    onExited: (clean) => {
      if (clean) {
        getCurrentWindow()
          .close()
          .catch((error: unknown) => bar.show(`Could not close the window: ${error}`));
      } else {
        stopped = true;
      }
    },
    onAttachLost: () => void bringUp(),
  });

  // Clicks on the padding or the bar would otherwise take the keyboard from the terminal.
  document.addEventListener("mousedown", (event) => {
    if (!(event.target instanceof Node && parent.contains(event.target))) {
      event.preventDefault();
    }
  });

  async function chooseSession(next: Connection, home: string): Promise<number> {
    // Kept across restarts, so a new window goes back to the same session.
    sessionUuid ??= lastSession();
    const live = (await next.list()).filter((item) => item.exited === null);
    // Session numbers start over in a new host, so the uuid is what says it is the same one.
    const existing = live.find((item) => item.uuid === sessionUuid) ?? live[0];
    let session: number;
    if (existing) {
      sessionUuid = existing.uuid;
      session = existing.session;
    } else {
      const uuid = crypto.randomUUID();
      const { cols, rows } = view.size;
      session = await next.create(uuid, "shell", home, cols, rows);
      sessionUuid = uuid;
    }
    rememberSession(sessionUuid);
    return session;
  }

  async function bringUp(): Promise<void> {
    if (bringingUp) return;
    bringingUp = true;
    // Finding the host starts it if it is gone, so a host that keeps crashing must not be
    // started again every second.
    let retry = RETRY_MS;
    for (;;) {
      connection?.close();
      let port: number | undefined;
      try {
        const host = await findHost();
        port = host.port;
        const next = new Connection({
          onLost: () => {
            if (stopped) return;
            bar.show(LOST_TEXT);
            void bringUp();
          },
          onError: (code, message) => {
            bar.note(
              code === "input_dropped"
                ? `Typing was dropped: ${message}`
                : `The host says: ${message}`,
            );
          },
        });
        connection = next;
        await next.open(host);
        const session = await chooseSession(next, host.home);
        await view.attach(next, session);
        bar.clear();
        bringingUp = false;
        return;
      } catch (error) {
        bar.show(describe(error, port));
        await sleep(retry);
        retry = Math.min(retry * 2, RETRY_MAX_MS);
      }
    }
  }

  bar.show("Looking for the host");
  view.focus();
  await bringUp();
}

void main();
