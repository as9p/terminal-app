import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Connection } from "./connection";

// The size the next fit lands on, standing in for the measured parent.
const nextSize = { cols: 80, rows: 24 };

class FakeTerminal {
  cols = 80;
  rows = 24;
  options: Record<string, unknown> = {};
  unicode = { activeVersion: "" };
  buffer = { active: { type: "normal" } };
  textarea: HTMLTextAreaElement | undefined;
  element: HTMLElement | undefined;
  keyHandler: ((event: KeyboardEvent) => boolean) | undefined;
  input = vi.fn();
  clear = vi.fn();
  selectAll = vi.fn();
  scrollPages = vi.fn();
  scrollToTop = vi.fn();
  scrollToBottom = vi.fn();
  write = vi.fn();
  focus = vi.fn();
  dispose = vi.fn();
  constructor() {
    terms.push(this);
  }
  loadAddon(addon: { term?: FakeTerminal }): void {
    addon.term = this;
  }
  attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean): void {
    this.keyHandler = handler;
  }
  open(parent: HTMLElement): void {
    this.element = document.createElement("div");
    this.textarea = document.createElement("textarea");
    this.element.append(this.textarea);
    parent.append(this.element);
  }
  onData() {
    return { dispose() {} };
  }
  onBinary() {
    return { dispose() {} };
  }
  onSelectionChange() {
    return { dispose() {} };
  }
  getSelection(): string {
    return "";
  }
}

const terms: FakeTerminal[] = [];

vi.mock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    term: FakeTerminal | undefined;
    fit(): void {
      if (!this.term) return;
      this.term.cols = nextSize.cols;
      this.term.rows = nextSize.rows;
    }
  },
}));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class {} }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class {} }));

let observed: ((entries: { contentRect: { width: number; height: number } }[]) => void) | undefined;

class FakeResizeObserver {
  constructor(callback: typeof observed) {
    observed = callback;
  }
  observe(): void {}
  disconnect(): void {}
}

const { TerminalView } = await import("./terminal");

function connection() {
  return {
    attach: vi.fn(() => Promise.resolve()),
    detach: vi.fn(),
    resize: vi.fn(),
    sendInput: vi.fn(),
    sendBinaryString: vi.fn(),
    ack: vi.fn(),
  };
}

function setup(attach = true) {
  const parent = document.createElement("div");
  document.body.append(parent);
  const view = new TerminalView(parent, { onSelection: vi.fn(), onExited: vi.fn(), onAttachLost: vi.fn() }, "dom");
  const term = terms[terms.length - 1];
  const conn = connection();
  if (attach) void view.attach(conn as unknown as Connection, 7);
  // The parent's box changes to this many cells, as a drag would.
  const resizeTo = (cols: number, rows: number, width = 600) => {
    nextSize.cols = cols;
    nextSize.rows = rows;
    observed?.([{ contentRect: { width, height: width ? 400 : 0 } }]);
    vi.advanceTimersByTime(16);
  };
  return { view, term, conn, resizeTo };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame"] });
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  nextSize.cols = 80;
  nextSize.rows = 24;
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.body.replaceChildren();
  terms.length = 0;
});

describe("terminal view", () => {
  it("the_hidden_textarea_asks_for_no_system_suggestions", () => {
    const { term, view } = setup(false);
    const area = term.textarea;
    expect(area?.getAttribute("autocorrect")).toBe("off");
    expect(area?.getAttribute("autocapitalize")).toBe("off");
    expect(area?.getAttribute("autocomplete")).toBe("off");
    expect(area?.getAttribute("writingsuggestions")).toBe("false");
    expect(area?.spellcheck).toBe(false);
    view.dispose();
  });

  describe("resize", () => {
    it("a_drag_sends_one_resize_once_the_size_settles", () => {
      const { conn, term, resizeTo, view } = setup();
      expect(conn.attach).toHaveBeenCalledWith(7, 80, 24, expect.anything());
      resizeTo(90, 24);
      // The view follows at once; the host waits.
      expect(term.cols).toBe(90);
      vi.advanceTimersByTime(50);
      resizeTo(100, 30);
      vi.advanceTimersByTime(50);
      resizeTo(110, 32);
      vi.advanceTimersByTime(50);
      expect(conn.resize).not.toHaveBeenCalled();
      vi.advanceTimersByTime(100);
      expect(conn.resize).toHaveBeenCalledTimes(1);
      expect(conn.resize).toHaveBeenCalledWith(7, 110, 32);
      vi.advanceTimersByTime(1000);
      expect(conn.resize).toHaveBeenCalledTimes(1);
      view.dispose();
    });

    it("a_size_back_to_the_sent_one_sends_nothing", () => {
      const { conn, resizeTo, view } = setup();
      resizeTo(90, 24);
      resizeTo(80, 24);
      vi.advanceTimersByTime(1000);
      expect(conn.resize).not.toHaveBeenCalled();
      view.dispose();
    });

    it("a_pane_shown_from_parked_sends_its_size_at_once", () => {
      const { conn, resizeTo, view } = setup();
      // Parked: no box, so the fit keeps the old size.
      resizeTo(80, 24, 0);
      resizeTo(120, 40);
      expect(conn.resize).toHaveBeenCalledWith(7, 120, 40);
      vi.advanceTimersByTime(1000);
      expect(conn.resize).toHaveBeenCalledTimes(1);
      // Later changes wait again.
      resizeTo(100, 40);
      expect(conn.resize).toHaveBeenCalledTimes(1);
      vi.advanceTimersByTime(100);
      expect(conn.resize).toHaveBeenLastCalledWith(7, 100, 40);
      view.dispose();
    });

    it("attaching_again_sends_the_size_with_the_attach_and_drops_the_wait", () => {
      const { conn, resizeTo, view } = setup();
      resizeTo(90, 30);
      void view.attach(conn as unknown as Connection, 7);
      expect(conn.attach).toHaveBeenLastCalledWith(7, 90, 30, expect.anything());
      vi.advanceTimersByTime(1000);
      expect(conn.resize).not.toHaveBeenCalled();
      view.dispose();
    });

    it("dispose_drops_a_waiting_resize", () => {
      const { conn, resizeTo, view } = setup();
      resizeTo(90, 30);
      view.dispose();
      vi.advanceTimersByTime(1000);
      expect(conn.resize).not.toHaveBeenCalled();
    });
  });

  describe("mac line keys", () => {
    type Mods = { meta?: boolean; alt?: boolean; ctrl?: boolean; shift?: boolean };
    function press(term: FakeTerminal, key: string, mods: Mods, type = "keydown", code = "") {
      const event = new KeyboardEvent(type, {
        key,
        code,
        metaKey: mods.meta ?? false,
        altKey: mods.alt ?? false,
        ctrlKey: mods.ctrl ?? false,
        shiftKey: mods.shift ?? false,
        cancelable: true,
      });
      const result = term.keyHandler?.(event);
      return { result, prevented: event.defaultPrevented };
    }

    const SENT: [string, Mods, string, string][] = [
      ["Backspace", { meta: true }, "Backspace", "\x15"],
      ["ArrowLeft", { meta: true }, "ArrowLeft", "\x01"],
      ["ArrowRight", { meta: true }, "ArrowRight", "\x05"],
      ["Delete", { meta: true }, "Delete", "\x0b"],
      ["z", { meta: true }, "KeyZ", "\x1f"],
      ["ArrowLeft", { alt: true }, "ArrowLeft", "\x1bb"],
      ["ArrowRight", { alt: true }, "ArrowRight", "\x1bf"],
      ["Backspace", { alt: true }, "Backspace", "\x1b\x7f"],
      ["Delete", { alt: true }, "Delete", "\x1bd"],
    ];

    it.each(SENT)("%s_with_%o_sends_its_bytes_on_keydown_only", (key, mods, code, bytes) => {
      const { term, view } = setup();
      const down = press(term, key, mods, "keydown", code);
      expect(down).toEqual({ result: false, prevented: true });
      expect(term.input).toHaveBeenCalledWith(bytes, true);
      expect(press(term, key, mods, "keyup", code).result).toBe(false);
      expect(term.input).toHaveBeenCalledTimes(1);
      view.dispose();
    });

    it("cmd_k_clears_then_asks_the_shell_to_redraw", () => {
      const { term, view } = setup();
      expect(press(term, "k", { meta: true }, "keydown", "KeyK").result).toBe(false);
      expect(term.clear).toHaveBeenCalledTimes(1);
      expect(term.input).toHaveBeenCalledWith("\x0c", true);
      expect(term.clear.mock.invocationCallOrder[0]).toBeLessThan(term.input.mock.invocationCallOrder[0]);
      view.dispose();
    });

    it("cmd_a_selects_all_and_cmd_arrows_home_end_scroll", () => {
      const { term, view } = setup();
      expect(press(term, "a", { meta: true }, "keydown", "KeyA")).toEqual({ result: false, prevented: true });
      expect(term.selectAll).toHaveBeenCalledTimes(1);
      press(term, "ArrowUp", { meta: true });
      expect(term.scrollPages).toHaveBeenLastCalledWith(-1);
      press(term, "ArrowDown", { meta: true });
      expect(term.scrollPages).toHaveBeenLastCalledWith(1);
      press(term, "Home", { meta: true });
      expect(term.scrollToTop).toHaveBeenCalledTimes(1);
      press(term, "End", { meta: true });
      expect(term.scrollToBottom).toHaveBeenCalledTimes(1);
      expect(term.input).not.toHaveBeenCalled();
      view.dispose();
    });

    it("on_the_alternate_screen_scroll_keys_do_nothing_and_cmd_k_only_redraws", () => {
      const { term, view } = setup();
      term.buffer.active.type = "alternate";
      for (const key of ["ArrowUp", "ArrowDown", "Home", "End"]) {
        expect(press(term, key, { meta: true }).result).toBe(false);
      }
      expect(term.scrollPages).not.toHaveBeenCalled();
      expect(term.scrollToTop).not.toHaveBeenCalled();
      expect(term.scrollToBottom).not.toHaveBeenCalled();
      press(term, "k", { meta: true }, "keydown", "KeyK");
      expect(term.clear).not.toHaveBeenCalled();
      expect(term.input).toHaveBeenCalledWith("\x0c", true);
      view.dispose();
    });

    it("right_option_acts_like_left_for_the_line_keys", () => {
      const { term, view } = setup();
      window.dispatchEvent(new KeyboardEvent("keydown", { code: "AltRight", altKey: true }));
      press(term, "ArrowLeft", { alt: true }, "keydown", "ArrowLeft");
      expect(term.input).toHaveBeenCalledWith("\x1bb", true);
      view.dispose();
    });

    it("other_keys_stay_with_xterm_or_the_app", () => {
      const { term, view } = setup();
      const passes: [string, Mods, string][] = [
        ["a", {}, "KeyA"],
        ["Backspace", {}, "Backspace"],
        // Fn+Delete: xterm already sends ESC [3~.
        ["Delete", {}, "Delete"],
        // Shift+PageUp: xterm already scrolls a page.
        ["PageUp", { shift: true }, "PageUp"],
        ["ArrowLeft", { meta: true, alt: true }, "ArrowLeft"],
        ["ArrowLeft", { meta: true, shift: true }, "ArrowLeft"],
        ["ArrowLeft", { alt: true, shift: true }, "ArrowLeft"],
        ["Backspace", { meta: true, ctrl: true }, "Backspace"],
        ["k", { meta: true, shift: true }, "KeyK"],
        ["b", { alt: true }, "KeyB"],
        ["t", { meta: true }, "KeyT"],
      ];
      for (const [key, mods, code] of passes) {
        expect(press(term, key, mods, "keydown", code).result, `${key} ${JSON.stringify(mods)}`).toBe(true);
      }
      // Copy and paste stay with the Edit menu.
      expect(press(term, "c", { meta: true }, "keydown", "KeyC")).toEqual({ result: false, prevented: false });
      expect(press(term, "v", { meta: true }, "keydown", "KeyV")).toEqual({ result: false, prevented: false });
      expect(term.input).not.toHaveBeenCalled();
      expect(term.clear).not.toHaveBeenCalled();
      expect(term.selectAll).not.toHaveBeenCalled();
      view.dispose();
    });
  });
});
