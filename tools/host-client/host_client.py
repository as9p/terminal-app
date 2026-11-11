"""Small command line client for the terminal host. Standard library only.

Talks to the host over a WebSocket on 127.0.0.1 using protocol version 3: JSON text
frames, and binary frames made of a 4-byte little-endian session number plus bytes.
"""

import argparse
import base64
import json
import os
import re
import socket
import struct
import sys
import threading
import time
import uuid

PROTO = 3
DEFAULT_PORT = 47211
APP_FOLDER = "terminal-app"
TOKEN_FILE = "host-token"
# An allowed origin. The host refuses any other origin that is present.
ORIGIN = "tauri://localhost"
HELLO_TIMEOUT = 5


class ClientError(Exception):
    """A problem to show the person as one plain line, without a traceback."""


def data_dir():
    """The same folder the host uses: TERM_APP_DATA, else the platform data dir."""
    override = os.environ.get("TERM_APP_DATA")
    if override:
        return override
    if sys.platform == "win32":
        base = os.environ.get("APPDATA") or os.path.join(
            os.path.expanduser("~"), "AppData", "Roaming"
        )
    elif sys.platform == "darwin":
        base = os.path.join(os.path.expanduser("~"), "Library", "Application Support")
    else:
        base = os.environ.get("XDG_DATA_HOME") or os.path.join(
            os.path.expanduser("~"), ".local", "share"
        )
    return os.path.join(base, APP_FOLDER)


def read_token(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return handle.read().strip()
    except FileNotFoundError:
        raise ClientError(
            f"no token file at {path}. Start the host once so it creates it, "
            "or pass --token-file."
        )
    except OSError as error:
        raise ClientError(f"cannot read the token file {path}: {error}")


class Connection:
    """A WebSocket client that has said hello and been accepted."""

    def __init__(self, port, token):
        self.port = port
        self.buffer = b""
        self.send_lock = threading.Lock()
        try:
            self.sock = socket.create_connection(("127.0.0.1", port), timeout=HELLO_TIMEOUT)
        except OSError:
            raise ClientError(
                f"cannot reach a host on port {port}. Is it running? "
                "Check --port or TERM_HOST_PORT."
            )
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self._upgrade()
        self.say({"type": "hello", "token": token, "proto": PROTO})
        try:
            frame = self.hear()
        except (EOFError, OSError):
            raise ClientError(
                "the host closed the connection after hello. The token is probably wrong "
                "or stale: check the token file in the host's data folder."
            )
        if not isinstance(frame, dict):
            raise ClientError("the host answered hello with something unexpected")
        if frame.get("type") == "error":
            if frame.get("code") == "proto_mismatch":
                raise ClientError(
                    f"version mismatch: this client speaks protocol {PROTO}, "
                    f"the host says: {frame.get('message')}"
                )
            raise ClientError(f"host error {frame.get('code')}: {frame.get('message')}")
        if frame.get("type") != "hello_ok" or frame.get("proto") != PROTO:
            raise ClientError(f"version mismatch: unexpected hello reply {frame}")
        self.host_version = frame.get("host_version")
        self.sock.settimeout(None)

    def _upgrade(self):
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            "GET / HTTP/1.1\r\n"
            f"Host: 127.0.0.1:{self.port}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
            f"Origin: {ORIGIN}\r\n\r\n"
        )
        try:
            self.sock.sendall(request.encode())
            while b"\r\n\r\n" not in self.buffer:
                chunk = self.sock.recv(4096)
                if not chunk:
                    raise ClientError("the host closed the connection during the upgrade")
                self.buffer += chunk
        except OSError as error:
            raise ClientError(f"the connection to the host failed: {error}")
        head, self.buffer = self.buffer.split(b"\r\n\r\n", 1)
        status = head.split(b"\r\n")[0].decode("latin-1")
        if " 101 " not in status:
            raise ClientError(f"the host refused the WebSocket upgrade ({status})")

    def _send(self, opcode, payload):
        mask = os.urandom(4)
        length = len(payload)
        if length < 126:
            header = struct.pack("!BB", 0x80 | opcode, 0x80 | length)
        elif length < 65536:
            header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, length)
        else:
            header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, length)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self.send_lock:
            self.sock.sendall(header + mask + masked)

    def say(self, frame):
        self._send(1, json.dumps(frame).encode())

    def say_bytes(self, session, data):
        self._send(2, struct.pack("<I", session) + data)

    def _need(self, count):
        while len(self.buffer) < count:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError
            self.buffer += chunk
        out, self.buffer = self.buffer[:count], self.buffer[count:]
        return out

    def hear(self):
        """Returns a dict for a JSON frame, or (session, bytes) for a binary one."""
        message = b""
        message_opcode = None
        while True:
            first, second = self._need(2)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._need(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._need(8))[0]
            payload = self._need(length)
            if opcode == 8:
                raise EOFError
            if opcode == 9:
                self._send(10, payload)
                continue
            if opcode == 10:
                continue
            if opcode != 0:
                message_opcode = opcode
            message += payload
            if not first & 0x80:
                continue
            if message_opcode == 1:
                return json.loads(message)
            if len(message) < 4:
                message, message_opcode = b"", None
                continue
            return struct.unpack("<I", message[:4])[0], message[4:]

    def close(self):
        try:
            self._send(8, b"")
        except OSError:
            pass
        self.sock.close()


def connect(args):
    token = read_token(args.token_file)
    return Connection(args.port, token)


def fetch_sessions(conn):
    conn.say({"type": "list"})
    while True:
        frame = conn.hear()
        if isinstance(frame, dict) and frame.get("type") == "sessions":
            return frame["items"]


def find_session(conn, number):
    for item in fetch_sessions(conn):
        if item["session"] == number:
            return item
    raise ClientError(f"no session {number}. Run the list command to see them.")


UNESCAPES = {"r": "\r", "n": "\n", "t": "\t", "e": "\x1b", "0": "\0", "\\": "\\"}
ESCAPE_PATTERN = re.compile(r"\\x([0-9a-fA-F]{2})|\\([rnte0\\])|[^\\]+|\\")


def decode_escapes(text):
    """Turns \\r \\n \\t \\e \\0 \\\\ and \\xNN into bytes. Other text stays as UTF-8."""
    out = bytearray()
    for match in ESCAPE_PATTERN.finditer(text):
        if match.group(1):
            out.append(int(match.group(1), 16))
        elif match.group(2):
            out += UNESCAPES[match.group(2)].encode()
        else:
            out += match.group(0).encode("utf-8")
    return bytes(out)


class Attachment:
    """Reads one attached session: prints output, acks what it reads, notes how it ended."""

    def __init__(self, conn, session, show_snapshot):
        self.conn = conn
        self.session = session
        self.show_snapshot = show_snapshot
        self.received = 0
        self.skip = 0
        self.ended = False

    def attach(self, cols, rows):
        self.conn.say({"type": "attach", "session": self.session, "cols": cols, "rows": rows})

    def handle(self, frame):
        """Returns False once the session is over for this client."""
        if isinstance(frame, tuple):
            session, data = frame
            if session != self.session:
                return True
            self.received += len(data)
            # Acks are the running total since attach, and the snapshot counts too.
            self.conn.say({"type": "ack", "session": self.session, "bytes": self.received})
            if self.skip:
                dropped = min(self.skip, len(data))
                self.skip -= dropped
                data = data[dropped:]
            if data:
                sys.stdout.buffer.write(data)
                sys.stdout.buffer.flush()
            return True
        kind = frame.get("type")
        if kind == "attached" and frame.get("session") == self.session:
            if not self.show_snapshot:
                self.skip = frame.get("snapshot_bytes", 0)
        elif kind == "exited" and frame.get("session") == self.session:
            note(f"session {self.session} exited")
            self.ended = True
            return False
        elif kind == "detached" and frame.get("session") == self.session:
            note(f"the host detached session {self.session}")
            self.ended = True
            return False
        elif kind == "error":
            note(f"host error {frame.get('code')}: {frame.get('message')}")
            if frame.get("code") in ("no_session", "not_attached"):
                self.ended = True
                return False
        return True

    def detach(self):
        if not self.ended:
            try:
                self.conn.say({"type": "detach", "session": self.session})
            except OSError:
                pass


def note(message):
    print(message, file=sys.stderr)


def cmd_list(args):
    conn = connect(args)
    items = fetch_sessions(conn)
    conn.close()
    if not items:
        print("no sessions")
        return
    print(f"{'SESSION':<8} {'KIND':<7} {'SIZE':<8} {'STATE':<12} TITLE")
    for item in items:
        exited = item["exited"]
        state = "running" if exited is None else ("exited" if exited else "failed")
        size = f"{item['cols']}x{item['rows']}"
        print(
            f"{item['session']:<8} {item['kind']:<7} {size:<8} {state:<12} {item['title'] or ''}"
        )


def cmd_new(args):
    conn = connect(args)
    conn.say(
        {
            "type": "create",
            "uuid": str(uuid.uuid4()),
            "kind": "shell",
            "cwd": args.cwd or os.path.expanduser("~"),
            "cols": args.cols,
            "rows": args.rows,
        }
    )
    while True:
        frame = conn.hear()
        if not isinstance(frame, dict):
            continue
        if frame.get("type") == "created":
            print(frame["session"])
            break
        if frame.get("type") == "error":
            raise ClientError(f"host error {frame.get('code')}: {frame.get('message')}")
    conn.close()


def cmd_send(args):
    conn = connect(args)
    item = find_session(conn, args.session)
    attachment = Attachment(conn, args.session, args.snapshot)
    attachment.attach(item["cols"], item["rows"])
    conn.say_bytes(args.session, decode_escapes(args.text))
    end = time.monotonic() + args.wait
    try:
        while True:
            left = end - time.monotonic()
            if left <= 0:
                break
            conn.sock.settimeout(left)
            try:
                frame = conn.hear()
            except socket.timeout:
                break
            if not attachment.handle(frame):
                break
    except EOFError:
        note("the host closed the connection")
    attachment.detach()
    conn.close()


def cmd_type(args):
    conn = connect(args)
    item = find_session(conn, args.session)
    attachment = Attachment(conn, args.session, not args.no_snapshot)
    attachment.attach(item["cols"], item["rows"])

    def read_output():
        try:
            while attachment.handle(conn.hear()):
                pass
        except (EOFError, OSError):
            if not (attachment.ended or closing.is_set()):
                note("the host closed the connection")
        attachment.ended = True

    closing = threading.Event()
    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    try:
        for line in sys.stdin:
            if attachment.ended:
                break
            conn.say_bytes(args.session, line.rstrip("\r\n").encode("utf-8") + b"\r")
    except KeyboardInterrupt:
        pass
    # Piped input ends at once, so give the last output a moment to arrive.
    deadline = time.monotonic() + args.wait
    while not attachment.ended and time.monotonic() < deadline:
        time.sleep(0.05)
    closing.set()
    attachment.detach()
    conn.close()


def cmd_kill(args):
    conn = connect(args)
    find_session(conn, args.session)
    conn.say({"type": "kill", "session": args.session})
    # A kill that fails answers with an error frame, so listen briefly.
    conn.sock.settimeout(0.5)
    try:
        while True:
            frame = conn.hear()
            if isinstance(frame, dict) and frame.get("type") == "error":
                raise ClientError(f"host error {frame.get('code')}: {frame.get('message')}")
    except (socket.timeout, EOFError):
        pass
    conn.close()
    print(f"asked the host to kill session {args.session}")


def build_parser():
    port_default = os.environ.get("TERM_HOST_PORT") or DEFAULT_PORT
    parser = argparse.ArgumentParser(description="Small client for the terminal host.")
    parser.add_argument("--port", type=int, default=int(port_default), help="host port")
    parser.add_argument(
        "--token-file",
        default=os.path.join(data_dir(), TOKEN_FILE),
        help="path to host-token (default: the host's data folder)",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("list", help="print sessions").set_defaults(run=cmd_list)

    new = sub.add_parser("new", help="start a shell session and print its number")
    new.add_argument("--cols", type=int, default=80)
    new.add_argument("--rows", type=int, default=24)
    new.add_argument("--cwd", help="start folder (default: your home folder)")
    new.set_defaults(run=cmd_new)

    send = sub.add_parser("send", help="type text into a session and print the output")
    send.add_argument("session", type=int)
    send.add_argument("text", help=r"text; \r \n \t \e \xNN \\ are decoded")
    send.add_argument("--wait", type=float, default=2.0, help="seconds to read output")
    send.add_argument("--snapshot", action="store_true", help="also print the screen snapshot")
    send.set_defaults(run=cmd_send)

    typer = sub.add_parser("type", help="interactive: stdin lines in, output out")
    typer.add_argument("session", type=int)
    typer.add_argument("--wait", type=float, default=1.0, help="seconds to read after stdin ends")
    typer.add_argument("--no-snapshot", action="store_true", help="skip the screen snapshot")
    typer.set_defaults(run=cmd_type)

    kill = sub.add_parser("kill", help="kill a session")
    kill.add_argument("session", type=int)
    kill.set_defaults(run=cmd_kill)
    return parser


def main():
    args = build_parser().parse_args()
    try:
        args.run(args)
    except ClientError as error:
        print(f"host_client: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    sys.exit(main())
