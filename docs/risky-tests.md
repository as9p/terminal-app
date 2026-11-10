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

## 3. Which Claude Code hook signals can drive session states

Tests run on 2026-11-10 on the Mac, Claude Code 2.1.322, model Haiku. Marks: **seen** means seen in a real run, **docs** means from the docs only. Doc pages: hooks reference `https://code.claude.com/docs/en/hooks` (sections SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Notification, SubagentStop, Stop, SessionEnd), CLI reference `https://code.claude.com/docs/en/cli-reference` (`--settings`, `--session-id`), subagents `https://code.claude.com/docs/en/sub-agents` ("Run subagents in foreground or background").

What I ran: a settings file with one hook per event that appends the event time and the raw JSON to a log. Real interactive `claude --settings <file> --session-id <uuid> --model haiku --permission-mode default` sessions, driven through a pseudo-terminal by `expect` (tmux is not installed), with `CLAUDECODE` and every `CLAUDE_*` variable unset by a wrapper script. `--permission-mode default` is needed on this Mac because the user settings set `defaultMode` to `auto`, and in auto mode no permission prompt appears (seen in a first run). Chats: a plain answer, a Write with a permission prompt, a Bash command with a permission prompt, a helper agent, a background helper. A second run turned fork mode off (`CLAUDE_CODE_FORK_SUBAGENT=0`) to get a helper that blocks the main turn. A third and fourth run registered only `PreToolUse` (matcher `Edit|Write|Agent`), `Notification`, `Stop` and `UserPromptSubmit`. Full commands and logs are in the task report.

Fields on every event (docs): `session_id`, `transcript_path`, `cwd`, `hook_event_name`, and mostly `permission_mode`, `prompt_id`, `effort`. Per event, seen in the logs:

| Event | Extra fields (seen) |
|---|---|
| `SessionStart` | `source`, `model` |
| `UserPromptSubmit` | `prompt` |
| `PreToolUse` | `tool_name`, `tool_input`, `tool_use_id` |
| `PostToolUse` | `tool_name`, `tool_input`, `tool_response`, `tool_use_id`, `duration_ms` |
| `PermissionRequest` | `tool_name`, `tool_input`, `permission_suggestions` |
| `Notification` | `notification_type`, `message` |
| `Stop` | `stop_hook_active`, `last_assistant_message`, `background_tasks`, `session_crons` |
| `SubagentStart` | `agent_id`, `agent_type` |
| `SubagentStop` | `agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message`, `background_tasks`, `session_crons` |
| `SessionEnd` | `reason` (`prompt_input_exit` when I quit) |

`Notification.notification_type` values (docs): `permission_prompt`, `idle_prompt`, `auth_success`, `auth_storage_failure`, `elicitation_dialog`, `elicitation_url_dialog`, `elicitation_complete`, `elicitation_response`, `agent_needs_input`, `agent_completed`, `quota_auto_resume_*`, `plugin_notification`. Only these two matter here. `permission_prompt` means waiting for permission, seen with message "Claude needs your permission". `idle_prompt` means waiting for input, seen with message "Claude is waiting for your input". Seen timing: `idle_prompt` came 60.1 s and 60.1 s after `Stop`, and only when nothing was typed. `permission_prompt` came 6.0 s after the prompt appeared. Docs say each keystroke defers it, so it is late and can be missed as the "blocked" trigger. The `PermissionRequest` event fires within 0.1 s of the prompt (seen, 0.06 s and 0.07 s) and is the better trigger. Nothing was seen of `elicitation_dialog` or `agent_needs_input`.

Rulings:

**(a) A helper of this session is still running when `Stop` fires. Seen.** Rule: at `Stop`, the session is still busy if `background_tasks` holds any entry with `type` equal to `subagent` and `status` equal to `running`. Then hold "done". The helper ends with `SubagentStop`, the main session gets a `UserPromptSubmit` whose `prompt` starts with `<task-notification>`, takes a short turn, and fires `Stop` again. That later `Stop` has `background_tasks` empty and is the real "done". Evidence: a `Stop` with `"background_tasks":[{"id":"aabf40971fc1053b7","type":"subagent","status":"running",...}]` arrived while the helper ran, and the last `Stop` of the same chat had `"background_tasks":[]`. A helper that ran to the end inside the turn (fork mode off) left `[]`. Cautions: (1) `SubagentStop` lists its own helper as running in `background_tasks`, so do not use that array. (2) The array also holds other task types. A helper's own background command appeared as `type` `shell`, and it was still listed at the last `Stop` while the helper itself was gone. So count `subagent` entries only, and do not count `shell`. Other types (`teammate`, `workflow`, `monitor`) are named in the docs and were not tested. (3) I did not see the race where a helper ends just before `Stop` and its report arrives after it. If it exists it shows as a quick done, then working, then done. Cheap guard that costs nothing: after a `Stop` with an empty array, wait 2 s and cancel the "done" if any event arrives.

**(b) Telling a helper's events from the main session's. Seen.** Helper events carry `agent_id` and `agent_type` (`PreToolUse`, `PostToolUse`, `SubagentStart`, `SubagentStop`). The main session's events never do, including the main session's own `PreToolUse` and `PostToolUse` for the `Agent` tool call, and `Stop`. `Stop` never fired for a helper, only `SubagentStop`. All events share the main `session_id`, so `session_id` alone does not separate them. A background helper's launch shows `tool_response.status` `async_launched`. In this version a helper that Claude starts from the main chat runs in the background by default (docs, sub-agents page, seen), so even a "wait for it" request ran in the background until fork mode was turned off. Agent team members: docs say they also get `agent_id` and `agent_type`. Not tested.

**(c) Edited file path. Seen.** `PreToolUse.tool_input.file_path` for both `Edit` and `Write`, absolute. `Edit` also has `old_string`, `new_string`, `replace_all`. `Write` has `content`. `PostToolUse` repeats `tool_input`.

**(d) `--settings` without touching `~/.claude`. Seen.** The hooks in the file ran, and the user's `~/.claude/settings.json` had the same checksum (`a826b666...`) before and after all runs. The user's own hooks kept running too (the screen showed "running UserPromptSubmit hooks... 0/3"). The CLI reference says the file overrides the same keys and keeps the others. One caveat for keys that are lists or objects: I saw it add the hooks, not replace them. Side effects not caused by `--settings` but by Claude itself: transcripts and per-session folders under `~/.claude`, and a trust entry per new folder in `~/.claude.json`.

**(e) Session id in hooks equals `--session-id`. Seen.** All 53 events of two runs carried exactly the id I passed. This also holds for the helper events.

**(f) What fires between approval and `Stop`. Seen.** With every event logged, the order for a Write and for a Bash was: `PreToolUse`, `PermissionRequest`, `Notification` (`permission_prompt`, 6.0 s later), approval, `PostToolUse`, `Stop`. `PreToolUse` fires before the prompt, not after it. With hooks only for `PreToolUse` (`Edit|Write|Agent`), `Notification`, `Stop`, `UserPromptSubmit`: after the Notification for the Write there was no event for 6.5 s until `Stop`, and after the Notification for the Bash none for 5.1 s until `Stop`. So yes, a "blocked" state set by `Notification` stays stuck until `Stop` with that set. `PreToolUse` on all tools would not fix it, because the approved tool's `PreToolUse` already fired before the prompt. It would only clear at the next tool call. Smallest hook set that clears "blocked" as soon as work resumes: `PostToolUse` with no matcher (fires after the approved tool finishes; it came 4.1 s after the Notification for the Write, when my script pressed Enter), together with the set trigger (`PermissionRequest`, or `Notification`) and `Stop`. Add `PostToolUseFailure` (docs only) so an approved tool that fails also clears it. Add `UserPromptSubmit` so a typed answer also clears it. Not tested: denying the prompt (docs say `PostToolUse` does not fire then, so the next event would be `Stop`), and a tool that runs very long after approval (blocked would show until it finishes).

Not established: team member and master worker events, `elicitation_dialog`, whether `Stop` fires after an interrupt (docs say it does not), and runs on the PC.
