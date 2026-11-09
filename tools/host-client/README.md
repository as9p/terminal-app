# Host client

A small command line client for the terminal host. It needs only Python 3.9 or newer and works on Windows and macOS. Use it to try the host by hand, or to let an agent drive a shell, without the app.

## Start a host for testing

Build and run the host from the repo root. Use your own port and data folder so you do not touch a real host:

    TERM_HOST_PORT=47388 TERM_APP_DATA=/tmp/host-test cargo run -p term-host

The host listens on 127.0.0.1, port 47211 unless `TERM_HOST_PORT` is set. It keeps its token in `host-token` inside the data folder. That folder is `TERM_APP_DATA` if set, else `terminal-app` in the platform data folder (macOS `~/Library/Application Support`, Windows `%APPDATA%`).

## Use it

Set the same two variables in the client's shell, or pass `--port` and `--token-file`.

    python3 host_client.py list
    python3 host_client.py new --cols 100 --rows 30    # prints the session number
    python3 host_client.py send 1 'echo hi\r' --wait 2   # decodes \r \n \t \e \xNN
    printf 'ls\npwd\n' | python3 host_client.py type 1   # each line plus Enter
    python3 host_client.py kill 1

Output is the raw terminal bytes, so colors and cursor codes show up as escape sequences. `send` hides the screen snapshot unless you add `--snapshot`. `type` ends on Ctrl+D or end of input.
