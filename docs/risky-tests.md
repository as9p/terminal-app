# Risky tests

Tests run on 2026-11-08 on the Mac, Claude Code 2.1.319, current account only. They settle two risks from DESIGN.md before later features rely on them.

## 1. Resume a chat from a different path (Mac, same machine)

What I ran, in a new scratch folder with `CLAUDECODE` and every `CLAUDE_CODE_*` variable unset, `-p`, `--model haiku`, 90 second limit:

1. In folder `a`: `claude -p --model haiku --output-format json "Remember the word plum. Reply ok."` Result `ok`, session id `52c6fbfb-4ab9-47f2-8c20-3f0d5fc35dbd`. The transcript appeared at `~/.claude/projects/<slug of a>/52c6fbfb-....jsonl`. The slug is the folder path with every character that is not a letter or digit replaced by `-`.
2. Moved the folder: `mv a b`. Folder `a` no longer existed.
3. In folder `b`, with nothing copied: `claude -p --model haiku --resume 52c6fbfb-4ab9-47f2-8c20-3f0d5fc35dbd "What word did I ask you to remember? One word."` Result `plum`, exit 0.

Result: resume under a different path works as is, by session id. No copy into the new path's folder under `~/.claude/projects/` was needed, so I did not run the second variant.

What the files showed:
- The resume did not create a transcript in the new path's folder. It only created an empty `memory` folder there.
- The new turn was appended to the original transcript in the old path's folder. Its entries record `cwd` as `b`, so the session follows the folder it is resumed from.

Not established:
- Whether `--resume` finds the transcript when the old path's folder under `~/.claude/projects/` is missing entirely, which is the case on another computer. This is the Mac to Windows case and stays untested until Task 8.
- Whether picking a chat from the `--resume` picker (no id) lists a session from another path. I only tested resume by id.
- Whether tool state tied to the old path (file edits, uncommitted changes) matters. The test had none.

## 2. Can Remote Control sessions be listed

Read only. No server was started.

- `claude remote-control --help`: options for naming, `--continue`, `--session-id <id>`, `--spawn`, `--capacity`, `--permission-mode`. No list option and no subcommand.
- `claude --help`: `claude agents` lists background sessions (`--bg`), which are not Remote Control sessions. `--remote-control [name]` starts an interactive session with Remote Control. No Remote Control list option found.
- Files: `ls ~/.claude` shows no file named for Remote Control. `~/.claude/sessions/<pid>.json` holds one small file per running local `claude` process. I read only the key names and the `bridgeSessionId` field.  Of 7 files, 4 have the key and 1 of those holds a value shaped like `session_...`. The other 3 have it null, and 3 files lack the key. (The key may only be written by newer or some session kinds.)

Answer: no command found that lists Remote Control sessions. One local file source exists: `~/.claude/sessions/*.json`, field `bridgeSessionId`, which appears to mark running local sessions connected to Remote Control (inferred from the name, not confirmed). It only covers this computer and only running processes.

Unknown, because I did not start a server:
- Whether a session spawned by a `claude remote-control` server writes a `bridgeSessionId` there. The one non-null file may come from interactive sessions started with `--remote-control` or `/remote-control`, I did not check which.
- Whether sessions started on claude.ai or the phone, and not on this computer, appear anywhere locally. Nothing suggests they do.
