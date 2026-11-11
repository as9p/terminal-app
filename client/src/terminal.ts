// One xterm.js terminal showing one host session. The parent element decides its size.

import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";

import type { Connection, SessionListener } from "./connection";
import { followLeftOption } from "./option-key";
import { fontFamily, fontSize, lineHeight, theme } from "./theme";
import { ZOOM_DEFAULT, spacingFor } from "./zoom";

const ACK_BYTES = 128 * 1024;
const ACK_QUIET_MS = 100;
const REATTACH_PAUSE_MS = 1000;
// Each host resize makes the shell redraw, so a drag sends only the size it ends on.
const RESIZE_SETTLE_MS = 100;
const SHIFT_ENTER = "\x1b\r";
// What Cmd or Option with a line-editing key sends, as kitty and iTerm do on a Mac.
const CMD_BACKSPACE = "\x15";
const CMD_LEFT = "\x01";
const CMD_RIGHT = "\x05";
const CMD_DELETE = "\x0b";
const CMD_Z = "\x1f";
const OPTION_LEFT = "\x1bb";
const OPTION_RIGHT = "\x1bf";
const OPTION_BACKSPACE = "\x1b\x7f";
const OPTION_DELETE = "\x1bd";
// Ctrl+L, so the shell draws its prompt again on the cleared screen.
const REDRAW = "\x0c";
// Full reset through the write queue, so it lands after output still waiting to be parsed.
const FULL_RESET = "\x1bc";
const STOPPED_LINE = "\r\n\x1b[2mThe program stopped unexpectedly.\x1b[0m\r\n";

export type Renderer = "webgl" | "dom";

// The natural cell width at 14 px in CSS pixels, from the stage 1 handoff (16.8 device pixels at
// ratio 2), for when the font cannot be measured.
const REFERENCE_CELL_CSS = 8.4;
let measureCanvas: HTMLCanvasElement | undefined;

/** The terminal font's natural cell width at 14 px in device pixels, before xterm rounds it. */
function referenceCellWidth(): number {
  try {
    measureCanvas ??= document.createElement("canvas");
    const context = measureCanvas.getContext("2d");
    if (context) {
      context.font = `${ZOOM_DEFAULT}px ${fontFamily}`;
      const width = context.measureText("W").width;
      if (width > 0) return width * devicePixelRatio;
    }
  } catch {
    // Falls back to the measured value below.
  }
  return REFERENCE_CELL_CSS * devicePixelRatio;
}

export type TerminalViewEvents = {
  /** The selection changed to this non-empty text. */
  onSelection(text: string): void;
  /** The session ended. On an unclean end the view has already said so on screen. */
  onExited(clean: boolean): void;
  /** Attaching again after `detached` failed, so the view shows nothing live any more. */
  onAttachLost(error: unknown): void;
  onTitle?(title: string): void;
};

/**
 * Call before creating the first view, so its first fit measures the real font. All four faces,
 * because xterm caches each glyph once: bold text drawn before its face loads would stay wrong.
 */
export async function terminalFontReady(): Promise<void> {
  const family = '"JetBrainsMono Nerd Font Mono"';
  const styles = ["", "bold ", "italic ", "italic bold "];
  try {
    await Promise.all(styles.map((style) => document.fonts.load(`${style}${fontSize}px ${family}`)));
  } catch {
    // The font stack falls back to monospace.
  }
}

export class TerminalView {
  private readonly parent: HTMLElement;
  private readonly events: TerminalViewEvents;
  private readonly term: Terminal;
  private readonly fitAddon = new FitAddon();
  private readonly observer: ResizeObserver;
  private readonly subscriptions: { dispose(): void }[] = [];
  private readonly stopOption: () => void;
  private readonly renderer: Renderer;
  private webgl: WebglAddon | undefined;

  private connection: Connection | undefined;
  private session = 0;
  private fitFrame = 0;
  // The size the host has, and a resize waiting for a drag to stop.
  private sentCols = 0;
  private sentRows = 0;
  private resizeTimer: ReturnType<typeof setTimeout> | undefined;
  // Parked (no box) at the last observed size; the next fit goes to the host at once.
  private parked = false;
  private resizeNow = false;

  // Bumped per attachment, so a write finishing late cannot count toward a newer total.
  private generation = 0;
  private taken = 0;
  private acked = 0;
  private lastWritten = 0;
  private ackTimer: ReturnType<typeof setTimeout> | undefined;
  private attachedAt = 0;
  private reattachTimer: ReturnType<typeof setTimeout> | undefined;

  /**
   * `renderer` is for speed runs: `dom` leaves out WebGL, so xterm draws with the DOM. `fontPx`
   * is the zoomed text size, so a new terminal opens at the saved size.
   */
  constructor(
    parent: HTMLElement,
    events: TerminalViewEvents,
    renderer: Renderer = "webgl",
    fontPx: number = fontSize,
  ) {
    this.parent = parent;
    this.events = events;
    this.renderer = renderer;
    this.term = new Terminal({
      allowProposedApi: true,
      cursorStyle: "bar",
      cursorBlink: false,
      drawBoldTextInBrightColors: false,
      fontFamily,
      fontSize: fontPx,
      // xterm rounds a glyph's width down and kitty rounds it up. The spacing makes up the
      // difference at each size: one device pixel at 14 px.
      letterSpacing: spacingFor(fontPx, referenceCellWidth()),
      lineHeight,
      scrollback: 20000,
      theme,
    });
    this.term.loadAddon(this.fitAddon);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = "11";
    this.term.attachCustomKeyEventHandler(this.onKey);
    this.term.open(parent);
    // Without these, macOS offers corrections and writing suggestions over the terminal.
    const textarea = this.term.textarea;
    if (textarea) {
      textarea.setAttribute("autocorrect", "off");
      textarea.setAttribute("autocapitalize", "off");
      textarea.setAttribute("autocomplete", "off");
      textarea.setAttribute("writingsuggestions", "false");
      textarea.spellcheck = false;
    }
    if (renderer === "webgl") this.loadWebgl();

    this.subscriptions.push(
      this.term.onData((data) => this.connection?.sendInput(this.session, data)),
      this.term.onBinary((data) => this.connection?.sendBinaryString(this.session, data)),
      this.term.onSelectionChange(() => {
        const text = this.term.getSelection();
        if (text) this.events.onSelection(text);
      }),
    );

    this.stopOption = followLeftOption((meta) => {
      this.term.options.macOptionIsMeta = meta;
    });

    this.fitAddon.fit();
    this.observer = new ResizeObserver(this.onResize);
    this.observer.observe(parent);
  }

  /** The size to pass to `create` before attaching. */
  get size(): { cols: number; rows: number } {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  /**
   * Shows `session`, resetting the terminal before its snapshot. Also the way to move to a new
   * connection, since the old one has nothing left to detach from. Rejects with what
   * `Connection.attach` rejects with.
   */
  attach(connection: Connection, session: number): Promise<void> {
    this.fitAddon.fit();
    this.connection = connection;
    this.session = session;
    this.startCounting();
    this.attachedAt = performance.now();
    // The attach carries the size, so nothing waiting needs to follow it.
    clearTimeout(this.resizeTimer);
    this.resizeTimer = undefined;
    this.sentCols = this.term.cols;
    this.sentRows = this.term.rows;
    return connection.attach(session, this.term.cols, this.term.rows, this.listener);
  }

  /** Stops showing the session and leaves its last screen in place. */
  detach(): void {
    this.connection?.detach(this.session);
    this.connection = undefined;
    this.startCounting();
  }

  focus(): void {
    this.term.focus();
  }

  /** Text zoom. The fit that follows sends the new column and row count to the host. */
  setFontSize(px: number): void {
    if (this.term.options.fontSize === px) return;
    this.term.options.letterSpacing = spacingFor(px, referenceCellWidth());
    this.term.options.fontSize = px;
    this.fitSoon();
  }

  /** Whether the view draws with WebGL now. */
  webglOn(): boolean {
    return this.webgl !== undefined;
  }

  /**
   * Loads or drops the view's WebGL context. Panes keeps only a few live (WEBGL_CAP); without one
   * xterm draws with the DOM. With the `dom` renderer there is never one.
   */
  setWebgl(on: boolean): void {
    if (!on) {
      this.dropWebgl();
    } else if (!this.webgl && this.renderer === "webgl") {
      this.loadWebgl();
    }
  }

  /** Only used with the `dim` setting on: the pane drops to 58 percent (styles.css). */
  setDimmed(on: boolean): void {
    this.parent.classList.toggle("dim", on);
  }

  /** One cell in CSS pixels, read from the drawn screen. Only for a divider drag ending. */
  cellSize(): { width: number; height: number } {
    const screen = this.term.element?.querySelector<HTMLElement>(".xterm-screen");
    const rect = screen?.getBoundingClientRect();
    if (rect && rect.width > 0 && this.term.cols > 0 && this.term.rows > 0) {
      return { width: rect.width / this.term.cols, height: rect.height / this.term.rows };
    }
    const px = this.term.options.fontSize ?? fontSize;
    return { width: px * 0.6, height: px * lineHeight };
  }

  dispose(): void {
    this.connection = undefined;
    this.startCounting();
    cancelAnimationFrame(this.fitFrame);
    clearTimeout(this.resizeTimer);
    clearTimeout(this.reattachTimer);
    this.observer.disconnect();
    this.stopOption();
    for (const subscription of this.subscriptions) subscription.dispose();
    // Disposing the terminal disposes the addon too.
    this.webgl = undefined;
    this.term.dispose();
    this.parent.replaceChildren();
  }

  private readonly listener: SessionListener = {
    onAttached: () => {
      this.startCounting();
      this.term.write(FULL_RESET);
    },
    onOutput: (bytes) => {
      const generation = this.generation;
      const length = bytes.length;
      this.term.write(bytes, () => this.written(generation, length));
    },
    onTitle: (title) => this.events.onTitle?.(title),
    onExited: (clean) => {
      this.sendAck();
      this.connection = undefined;
      this.startCounting();
      if (!clean) this.term.write(STOPPED_LINE);
      this.events.onExited(clean);
    },
    onDetached: () => {
      const connection = this.connection;
      if (!connection) return;
      // A host that drops this client straight after every attach must not cause a tight loop.
      const pause =
        performance.now() - this.attachedAt < REATTACH_PAUSE_MS ? REATTACH_PAUSE_MS : 0;
      this.reattachTimer = setTimeout(() => {
        if (this.connection !== connection) return;
        this.attach(connection, this.session).catch((error: unknown) => {
          this.events.onAttachLost(error);
        });
      }, pause);
    },
  };

  private startCounting(): void {
    this.generation++;
    this.taken = 0;
    this.acked = 0;
    clearTimeout(this.ackTimer);
    this.ackTimer = undefined;
  }

  private written(generation: number, length: number): void {
    if (generation !== this.generation) return;
    this.taken += length;
    this.lastWritten = performance.now();
    if (this.taken - this.acked >= ACK_BYTES) {
      this.sendAck();
    } else if (this.ackTimer === undefined) {
      this.ackTimer = setTimeout(this.ackWhenQuiet, ACK_QUIET_MS);
    }
  }

  // One timer that checks again beats clearing and setting a timer on every chunk of output.
  private readonly ackWhenQuiet = (): void => {
    const quiet = performance.now() - this.lastWritten;
    if (quiet < ACK_QUIET_MS) {
      this.ackTimer = setTimeout(this.ackWhenQuiet, ACK_QUIET_MS - quiet);
      return;
    }
    this.ackTimer = undefined;
    this.sendAck();
  };

  private sendAck(): void {
    if (this.taken > this.acked) {
      this.acked = this.taken;
      this.connection?.ack(this.session, this.acked);
    }
  }

  private readonly onResize = (entries: ResizeObserverEntry[]): void => {
    const box = entries[entries.length - 1]?.contentRect;
    const parked = !box || box.width === 0 || box.height === 0;
    if (this.parked && !parked) this.resizeNow = true;
    this.parked = parked;
    this.fitSoon();
  };

  // The view follows every frame; the host hears once the size stops changing.
  private readonly fitSoon = (): void => {
    if (this.fitFrame) return;
    this.fitFrame = requestAnimationFrame(() => {
      this.fitFrame = 0;
      const { cols, rows } = this.term;
      this.fitAddon.fit();
      if (this.resizeNow) {
        this.resizeNow = false;
        clearTimeout(this.resizeTimer);
        this.sendResize();
      } else if (this.term.cols !== cols || this.term.rows !== rows) {
        clearTimeout(this.resizeTimer);
        this.resizeTimer =
          this.term.cols === this.sentCols && this.term.rows === this.sentRows
            ? undefined
            : setTimeout(this.sendResize, RESIZE_SETTLE_MS);
      }
    });
  };

  private readonly sendResize = (): void => {
    this.resizeTimer = undefined;
    const { cols, rows } = this.term;
    if (!this.connection || (cols === this.sentCols && rows === this.sentRows)) return;
    this.sentCols = cols;
    this.sentRows = rows;
    this.connection.resize(this.session, cols, rows);
  };

  private loadWebgl(): void {
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        if (this.webgl !== webgl) return;
        this.dropWebgl();
        // On screen it draws with WebGL again at once. A parked pane (display: none, so it has no
        // boxes) gets one back when it is shown.
        if (this.parent.getClientRects().length > 0) this.loadWebgl();
      });
      this.term.loadAddon(webgl);
      this.webgl = webgl;
    } catch {
      // No WebGL here. xterm's default renderer stays in place.
    }
  }

  private dropWebgl(): void {
    const webgl = this.webgl;
    this.webgl = undefined;
    webgl?.dispose();
  }

  private readonly onKey = (event: KeyboardEvent): boolean => {
    const noOtherModifier = !event.altKey && !event.ctrlKey && !event.metaKey;
    if (event.key === "Enter" && event.shiftKey && noOtherModifier) {
      if (event.type === "keydown") {
        // Also stops the keypress and a newline landing in xterm's hidden textarea.
        event.preventDefault();
        this.term.input(SHIFT_ENTER, true);
      }
      return false;
    }
    // Plain typing leaves here, before any of the Mac line keys below.
    if (!event.metaKey && !event.altKey) return true;
    // Left to the browser, so the Edit menu copies and pastes and the shell never sees them.
    if (event.metaKey && (event.code === "KeyC" || event.code === "KeyV")) return false;
    // Exactly one of Cmd or Option. Cmd+Option+arrows move between panes (keys.ts).
    if (event.ctrlKey || event.shiftKey || event.metaKey === event.altKey) return true;
    const run = event.metaKey ? this.cmdKey(event) : this.optionKey(event.key);
    if (run === undefined) return true;
    if (event.type === "keydown") {
      // Also stops the browser's own select all, and a character landing in the textarea.
      event.preventDefault();
      run();
    }
    return false;
  };

  // Either Option key, unlike Option with a letter, which follows option-key.ts.
  private optionKey(key: string): (() => void) | undefined {
    switch (key) {
      case "ArrowLeft":
        return () => this.term.input(OPTION_LEFT, true);
      case "ArrowRight":
        return () => this.term.input(OPTION_RIGHT, true);
      case "Backspace":
        return () => this.term.input(OPTION_BACKSPACE, true);
      case "Delete":
        return () => this.term.input(OPTION_DELETE, true);
    }
    return undefined;
  }

  private cmdKey(event: KeyboardEvent): (() => void) | undefined {
    // A full-screen program owns the screen, so there is no scrollback to move or clear.
    const alternate = () => this.term.buffer.active.type === "alternate";
    switch (event.key) {
      case "Backspace":
        return () => this.term.input(CMD_BACKSPACE, true);
      case "ArrowLeft":
        return () => this.term.input(CMD_LEFT, true);
      case "ArrowRight":
        return () => this.term.input(CMD_RIGHT, true);
      case "Delete":
        return () => this.term.input(CMD_DELETE, true);
      case "ArrowUp":
        return () => {
          if (!alternate()) this.term.scrollPages(-1);
        };
      case "ArrowDown":
        return () => {
          if (!alternate()) this.term.scrollPages(1);
        };
      case "Home":
        return () => {
          if (!alternate()) this.term.scrollToTop();
        };
      case "End":
        return () => {
          if (!alternate()) this.term.scrollToBottom();
        };
    }
    // Letters by what they type, as the menu bar matches them, by position on a non-Latin layout.
    const { key, code } = event;
    let letter = "";
    if (key.length === 1 && key.charCodeAt(0) < 0x80) letter = key.toLowerCase();
    else if (code.startsWith("Key")) letter = code.slice(3).toLowerCase();
    switch (letter) {
      case "z":
        return () => this.term.input(CMD_Z, true);
      case "a":
        return () => this.term.selectAll();
      case "k":
        return () => {
          if (!alternate()) this.term.clear();
          this.term.input(REDRAW, true);
        };
    }
    return undefined;
  }
}
