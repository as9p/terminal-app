// One WebSocket to the host. It knows nothing about xterm or how many sessions are on screen.

const PROTO = 1;
const HEADER_BYTES = 4;
// Long enough for a busy host, short enough that a silent port does not look like a frozen app.
const HELLO_TIMEOUT_MS = 5000;

export type HostAddress = { port: number; token: string };

export type SessionKind = "shell" | "claude";

export type SessionItem = {
  session: number;
  uuid: string;
  kind: string;
  title: string | null;
  exited: boolean | null;
  cols: number;
  rows: number;
};

/** Why a connection could not be opened, or that it ended. */
export type FailureKind = "proto_mismatch" | "token_refused" | "not_our_host" | "lost";

export class ConnectionFailure extends Error {
  readonly kind: FailureKind;

  constructor(kind: FailureKind) {
    super(kind);
    this.kind = kind;
  }
}

/** An `error` frame that answers a request. */
export class HostError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

/** What happens to one attached session. A listener is dropped after onExited or onDetached. */
export type SessionListener = {
  onAttached(snapshotBytes: number): void;
  /** `bytes` is a view of the received frame, not a copy. */
  onOutput(bytes: Uint8Array): void;
  onTitle(title: string): void;
  onExited(clean: boolean): void;
  onDetached(): void;
};

export type ConnectionEvents = {
  /** The socket ended after a good hello. Not called after `close()`. */
  onLost(): void;
  /** An `error` frame that answers no waiting request. */
  onError(code: string, message: string): void;
};

type HostFrame =
  | { type: "hello_ok"; proto: number; host_version: string }
  | { type: "sessions"; items: SessionItem[] }
  | { type: "created"; session: number; uuid: string }
  | { type: "attached"; session: number; snapshot_bytes: number }
  | { type: "title"; session: number; title: string }
  | { type: "exited"; session: number; clean: boolean }
  | { type: "detached"; session: number }
  | { type: "layout"; json: string | null }
  | { type: "error"; code: string; message: string };

type Waiting = {
  // `error` frames carry no request id. The host answers frames in order, so an error goes to
  // the oldest request that can fail with that code.
  codes: readonly string[];
  matches(frame: HostFrame): boolean;
  settle(frame: HostFrame): void;
  reject(error: Error): void;
};

const LIST_CODES: readonly string[] = [];
const LAYOUT_CODES: readonly string[] = [];
const CREATE_CODES = ["bad_request", "bad_kind", "bad_cwd", "bad_size", "spawn_failed"];
const ATTACH_CODES = ["no_session", "bad_size"];

export class Connection {
  private ws: WebSocket | undefined;
  private ready = false;
  private readonly events: ConnectionEvents;
  private readonly listeners = new Map<number, SessionListener>();
  private waiting: Waiting[] = [];
  // Typing should allocate as little as possible, and send copies the bytes before it returns.
  private frame = new Uint8Array(4096);
  private readonly encoder = new TextEncoder();

  constructor(events: ConnectionEvents) {
    this.events = events;
  }

  /** Connects and says hello. Rejects with a ConnectionFailure. */
  open(address: HostAddress): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(`ws://127.0.0.1:${address.port}`);
      ws.binaryType = "arraybuffer";
      this.ws = ws;

      let opened = false;
      let settled = false;
      const fail = (kind: FailureKind) => {
        if (settled) return;
        settled = true;
        clearTimeout(deadline);
        ws.close();
        reject(new ConnectionFailure(kind));
      };
      const deadline = setTimeout(() => fail("not_our_host"), HELLO_TIMEOUT_MS);

      ws.onopen = () => {
        opened = true;
        ws.send(JSON.stringify({ type: "hello", token: address.token, proto: PROTO }));
      };
      ws.onmessage = (event: MessageEvent<string | ArrayBuffer>) => {
        const frame = parseFrame(event.data);
        if (frame?.type === "error" && frame.code === "proto_mismatch") {
          fail("proto_mismatch");
        } else if (frame?.type !== "hello_ok") {
          fail("not_our_host");
        } else if (frame.proto !== PROTO) {
          fail("proto_mismatch");
        } else {
          settled = true;
          clearTimeout(deadline);
          this.ready = true;
          ws.onmessage = this.onMessage;
          resolve();
        }
      };
      ws.onclose = () => {
        if (this.ws !== ws) return;
        if (this.ready) {
          this.onClosed();
        } else {
          // The host closes without a word on a wrong token. A socket that never opened, just
          // after host_connection found the port answering, was not a WebSocket server of ours.
          fail(opened ? "token_refused" : "not_our_host");
        }
      };
    });
  }

  /** Ends the connection on purpose. Waiting requests reject and `onLost` is not called. */
  close(): void {
    const ws = this.ws;
    this.ws = undefined;
    this.ready = false;
    this.listeners.clear();
    this.rejectWaiting();
    ws?.close();
  }

  list(): Promise<SessionItem[]> {
    return this.request(
      { type: "list" },
      LIST_CODES,
      (frame) => frame.type === "sessions",
      (frame, resolve: (items: SessionItem[]) => void) => {
        if (frame.type === "sessions") resolve(frame.items);
      },
    );
  }

  /** The layout last given to `setLayout`, kept by the host across restarts, or null. */
  layout(): Promise<string | null> {
    return this.request(
      { type: "layout_get" },
      LAYOUT_CODES,
      (frame) => frame.type === "layout",
      (frame, resolve: (json: string | null) => void) => {
        if (frame.type === "layout") resolve(frame.json);
      },
    );
  }

  /** The host never looks inside `json`. Dropped if the socket is not open. */
  setLayout(json: string): void {
    this.sendText({ type: "layout_set", json });
  }

  /** Resolves with the session number. Rejects with a HostError or a ConnectionFailure. */
  create(
    uuid: string,
    kind: SessionKind,
    cwd: string,
    cols: number,
    rows: number,
  ): Promise<number> {
    return this.request(
      { type: "create", uuid, kind, cwd, cols, rows },
      CREATE_CODES,
      (frame) => frame.type === "created" && frame.uuid === uuid,
      (frame, resolve: (session: number) => void) => {
        if (frame.type === "created") resolve(frame.session);
      },
    );
  }

  /**
   * Resolves when `attached` arrives, after `listener.onAttached`. The snapshot follows as the
   * next `onOutput`. Rejects with a HostError or a ConnectionFailure.
   */
  attach(session: number, cols: number, rows: number, listener: SessionListener): Promise<void> {
    this.listeners.set(session, listener);
    return this.request(
      { type: "attach", session, cols, rows },
      ATTACH_CODES,
      (frame) =>
        (frame.type === "attached" || frame.type === "exited" || frame.type === "detached") &&
        frame.session === session,
      (frame, resolve: () => void, reject) => {
        if (frame.type === "detached") {
          reject(new HostError("detached", "the host dropped this session while attaching"));
        } else {
          resolve();
        }
      },
    );
  }

  detach(session: number): void {
    this.listeners.delete(session);
    this.sendText({ type: "detach", session });
  }

  resize(session: number, cols: number, rows: number): void {
    this.sendText({ type: "resize", session, cols, rows });
  }

  kill(session: number): void {
    this.sendText({ type: "kill", session });
  }

  /** `bytes` is the running total taken in since `attached`, snapshot included. */
  ack(session: number, bytes: number): void {
    this.sendText({ type: "ack", session, bytes });
  }

  /** Sends typed text as UTF-8, synchronously. Dropped if the socket is not open. */
  sendInput(session: number, text: string): void {
    const ws = this.ws;
    if (!this.ready || ws === undefined || ws.readyState !== WebSocket.OPEN) return;
    const frame = this.frameFor(session, text.length * 3);
    const { written } = this.encoder.encodeInto(text, frame.subarray(HEADER_BYTES));
    ws.send(frame.subarray(0, HEADER_BYTES + written));
  }

  /** Sends a string whose characters are single bytes, as xterm's onBinary gives them. */
  sendBinaryString(session: number, text: string): void {
    const ws = this.ws;
    if (!this.ready || ws === undefined || ws.readyState !== WebSocket.OPEN) return;
    const frame = this.frameFor(session, text.length);
    for (let i = 0; i < text.length; i++) {
      frame[HEADER_BYTES + i] = text.charCodeAt(i);
    }
    ws.send(frame.subarray(0, HEADER_BYTES + text.length));
  }

  private frameFor(session: number, room: number): Uint8Array<ArrayBuffer> {
    if (HEADER_BYTES + room > this.frame.length) {
      this.frame = new Uint8Array(HEADER_BYTES + room);
    }
    const frame = this.frame;
    frame[0] = session & 0xff;
    frame[1] = (session >>> 8) & 0xff;
    frame[2] = (session >>> 16) & 0xff;
    frame[3] = (session >>> 24) & 0xff;
    return frame;
  }

  private sendText(frame: object): void {
    if (this.ready && this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(frame));
    }
  }

  private request<T>(
    frame: object,
    codes: readonly string[],
    matches: (frame: HostFrame) => boolean,
    settle: (
      frame: HostFrame,
      resolve: (value: T) => void,
      reject: (error: Error) => void,
    ) => void,
  ): Promise<T> {
    return new Promise((resolve, reject) => {
      if (!this.ready || this.ws?.readyState !== WebSocket.OPEN) {
        reject(new ConnectionFailure("lost"));
        return;
      }
      this.waiting.push({
        codes,
        matches,
        settle: (answer) => settle(answer, resolve, reject),
        reject,
      });
      this.ws.send(JSON.stringify(frame));
    });
  }

  private onMessage = (event: MessageEvent<string | ArrayBuffer>): void => {
    const data = event.data;
    if (typeof data !== "string") {
      if (data.byteLength < HEADER_BYTES) return;
      const session = new DataView(data).getUint32(0, true);
      this.listeners.get(session)?.onOutput(new Uint8Array(data, HEADER_BYTES));
      return;
    }
    const frame = parseFrame(data);
    if (frame) this.onTextFrame(frame);
  };

  private onTextFrame(frame: HostFrame): void {
    switch (frame.type) {
      case "attached":
        this.listeners.get(frame.session)?.onAttached(frame.snapshot_bytes);
        break;
      case "title":
        this.listeners.get(frame.session)?.onTitle(frame.title);
        break;
      case "exited": {
        const listener = this.listeners.get(frame.session);
        this.listeners.delete(frame.session);
        listener?.onExited(frame.clean);
        break;
      }
      case "detached": {
        const listener = this.listeners.get(frame.session);
        this.listeners.delete(frame.session);
        // A pending attach hears about this as a rejection, so only a running view is told.
        if (!this.takeWaiting(frame)) listener?.onDetached();
        return;
      }
      case "error": {
        const index = this.waiting.findIndex((waiting) => waiting.codes.includes(frame.code));
        if (index >= 0) {
          const [waiting] = this.waiting.splice(index, 1);
          waiting.reject(new HostError(frame.code, frame.message));
        } else {
          this.events.onError(frame.code, frame.message);
        }
        return;
      }
      default:
        break;
    }
    this.takeWaiting(frame);
  }

  private takeWaiting(frame: HostFrame): boolean {
    const index = this.waiting.findIndex((waiting) => waiting.matches(frame));
    if (index < 0) return false;
    const [waiting] = this.waiting.splice(index, 1);
    waiting.settle(frame);
    return true;
  }

  private onClosed(): void {
    this.ws = undefined;
    this.ready = false;
    this.listeners.clear();
    this.rejectWaiting();
    this.events.onLost();
  }

  private rejectWaiting(): void {
    const waiting = this.waiting;
    this.waiting = [];
    for (const request of waiting) {
      request.reject(new ConnectionFailure("lost"));
    }
  }
}

function parseFrame(data: string | ArrayBuffer): HostFrame | undefined {
  if (typeof data !== "string") return undefined;
  try {
    return JSON.parse(data) as HostFrame;
  } catch {
    return undefined;
  }
}
