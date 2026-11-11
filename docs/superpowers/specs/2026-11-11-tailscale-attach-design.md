# Tailscale attach: each computer's app opens the other computer's terminals

Date: 2026-11-11. Status: draft, waiting for Alex's approval. Nothing is built before that.

Lines marked **(Alex)** are things Alex said. Lines marked **(my call)** are choices made while writing this, and Alex can overrule any of them.

## What this is for

Alex runs many Claude Code sessions at once, on a Mac and on a Windows PC. Today that only works across both computers in Orca. The goal is that Snowflake on the Mac shows the PC's sessions in its sidebar and a click opens the real terminal, typed into as if it were local, and the same from the PC towards the Mac.

- Both computers run a host, and each computer's app can open the other's sessions. **(Alex: "i want them both to be hosts")**
- This comes before account switching in the build order. **(Alex: "tailscale higher than account switch")**
- It must be the real terminal, not the Claude app's view of a session. Claude Code's Remote Control stays as a backup route and is not part of this work. **(Alex)**
- Clients attach to any host over Tailscale, and only devices on Alex's tailnet can connect. Each device pairs once and gets its own token, so being on the tailnet is not enough by itself. **(Alex, in `DESIGN.md`)**
- No SSH is needed for any of this. SSH stays what `DESIGN.md` says it is: the way to restart a host that has died, built later.

## What exists today

Read from the code on 2026-11-11.

- The host (`host/`, crate `term-host`) listens only on `127.0.0.1`, port 47211. One token in the file `host-token` lets a client in. The first message on a connection must be `hello` with that token.
- Anyone holding the token can start a shell, so a token is the whole computer. That is by design.
- The host reports only its protocol version and its own version. It does not report a name, an id or which system it runs on.
- Session numbers start from 1 on every host, and group keys are built from local folder paths. Nothing is unique across two hosts.
- A terminal's size is set by whichever client attached or resized last.
- The host starts at login on the Mac (a launchd service). On Windows that part is a stub. The host itself builds and passes its tests on Windows.
- The app (`client/`) opens one connection from the web page to `ws://127.0.0.1:<port>`. Its security policy allows only that address. One tree, one "connected" state, one current tab.
- On this Mac, Tailscale is the App Store build. Its command line tool is at `/Applications/Tailscale.app/Contents/MacOS/Tailscale`, it is not on the PATH, and it answers `ip -4` and `status --json`. The tailnet has this Mac, one Windows PC and one phone, with MagicDNS (Tailscale's own device names ending in `.ts.net`) turned on.

## Goals

1. From the app on one computer, see every session on the other computer and open, type into, scroll and close any of them.
2. Start a new terminal on the other computer from this one.
3. Pair two computers once, in under a minute, with no file copied by hand.
4. A terminal shown on two computers at once never gets resized by the one Alex is not typing in.
5. The PC's host is running after the PC starts and Alex has logged in, without Alex opening the app.
6. When the other computer is off or unreachable, this computer's own sessions work exactly as before.

## Not in this work

- Restarting a dead host from the other computer (needs SSH), moving a session between computers, Claude reaching other computers through a command line tool, the Remote Control list in the sidebar. These are the next items in `DESIGN.md`'s build order.
- Cmd+Shift+N ("new tab on the other computer in the same project"). The plus button on a computer's row covers starting a terminal there for now. **(my call)**
- The phone app, HTTPS, passkeys.
- Account switching.
- The native engine. This is built on today's app. See "What carries over to the native app".

## What would make this a failure even if it works

- The host can be reached from the local network or the internet, or by a tailnet device that was never paired.
- Turning this on changes or slows local use when the other computer is away.
- Opening a PC session from the Mac scrambles what the PC's own window shows.
- Pairing needs Alex to copy a token file between computers.

## How it works

### The host listens on the Tailscale address too

- A setting on each computer, off by default, called "Let my other computers connect". **(my call)**
- When it is on, the host opens a second listener on this computer's own Tailscale address (a `100.x` address), same port number. It never listens on every address. The `127.0.0.1` listener does not change.
- The host finds its Tailscale address by running Tailscale's command line tool (`ip -4`). It looks for the tool in the known install places for Mac and Windows and then on the PATH. If Tailscale is not up yet, it tries again every few seconds, and it follows the address if it changes. **(my call: bind to the address instead of `tailscale serve`, because it needs no Tailscale setup step by Alex and the host can see who is connecting)**
- On the second listener the host also refuses any connection whose source address is outside Tailscale's range (`100.64.0.0/10`). It does this when the connection arrives, before any WebSocket handshake. This is a second check, not the main one.
- The switch belongs to one Tailscale network. When Alex turns it on, the host records which tailnet this computer is signed in to. It opens the second listener only while Tailscale reports that same tailnet. If the computer is later signed in to a different tailnet, the listener closes and Settings shows "paused" with the reason, until Alex turns the switch on again for the new one. **(my call, from the security check: signing in to another network must never expose this computer there)**
- Traffic between two tailnet devices is encrypted by Tailscale end to end, also when relayed, so the connection itself stays plain WebSocket.
- Windows needs Tailscale's "Allow incoming connections" turned on. The pairing screen says so when a connection cannot be made.
- Windows' own firewall may ask for administrator approval the first time a program listens on an address other than `127.0.0.1`. This is reasoned, not yet seen. The plan proves it on the PC first. If a firewall rule is needed, it is one rule added once at install, with one administrator prompt.

### Tokens: one per paired computer

- The existing `host-token` works only on the `127.0.0.1` listener. It is refused on the Tailscale listener.
- Each paired computer gets its own token. The host keeps only a hash of it, with the computer's name, its Tailscale address at pairing, when it was paired and when it last connected.
- A token is accepted only from the Tailscale address it was paired from. Tailscale ties an address to one device, so a token that leaks is useless from any other device. If a computer's Tailscale address ever changes, it is paired again. **(my call, from the security check)**
- Alex can see the list of paired computers and remove one. Removing one cuts its open connections at once. Checking a token and registering the new connection happen under the same lock as removal, so a connection can never slip in while its computer is being removed.
- A remote connection can do what a local one can with sessions and the tree. It cannot pair other computers, list or remove paired computers, read any token, or change the setting. Those are accepted only on the `127.0.0.1` listener. **(my call)** This stops mistakes and simple misuse. It is not a wall: see Risks.
- Limits on the Tailscale listener: a first message larger than about 4 KiB is refused, a cap on connections that have not said `hello` yet (counted per source address as well as in total), a cap on open connections, and a pause for a source address after 5 failed tries in a minute. **(my call on the numbers; the plan may tune them)**
- The host writes a log line for each pairing, each refused try and each remote connect and disconnect: time, computer name, address. Never a token or a code.

### Pairing: one code, both directions

1. On the computer to be reached (say the PC), Alex opens Settings and presses "Pair a computer". The app shows a code of 8 letters and digits. The code works once, for 5 minutes, and dies after 5 wrong tries.
2. On the other computer (the Mac), Alex presses "Add a computer". The app lists only the tailnet's computers that belong to the same Tailscale user as this one, taken from Tailscale's command line tool, so there is no address to type and no shared or stranger's device to pick by mistake. Alex picks the PC and types the code.
3. The Mac's app connects to the PC's Tailscale listener and sends the code, the Mac's name, the Mac's host id and its protocol version. No token is sent yet.
4. The PC's host checks that the source address is, right now, a device of the same Tailscale user, then the protocol version, then the code. If all hold, it stores the Mac as a paired computer bound to that address and answers with its own name, its host id and a fresh token for the Mac to use. If not, it answers with the reason (no code waiting, wrong code, code expired, or different versions, naming both).
5. Only after that answer, and only if the Mac's own switch is on, the Mac's app asks its own host for a fresh token that the Mac's host will accept from the PC's address, and sends it on the same connection as a second message. The PC's host stores it. The Mac's app hands the PC's answer to its own host, which stores it. Both hosts now hold what they need, so both directions work after one code. **(my call)**
6. Both screens then show who they paired with: the other computer's name, its Tailscale name and its address, with a "Not mine, remove" button. A wrong computer is caught at once.

- If the Mac's own switch is off at step 5, the pairing is one way (the Mac reaches the PC, not the reverse), and "Add a computer" says so before Alex types the code and offers to turn the switch on first. A computer never hands out a token for reaching itself while its switch is off. Making it two way later means pairing once from the other side. **(my call)**
- Tailscale guarantees which device answers at an address. The code proves to the PC that the person at the Mac is the person looking at the PC's screen.
- Only apps dial out. A host never connects to another host. The PC's app later asks the PC's host for its list of computers and connects to the Mac itself.
- The code is sent in a pairing message of its own, which is the only thing besides `hello` that a connection may send first. It is accepted only on the Tailscale listener and only while a code is waiting.
- A host refuses to pair with a computer that reports the host's own id.

- If the picked computer has not turned on "Let my other computers connect", step 3 fails and the app says which switch to turn on, on which computer.
- The tokens for reaching other computers are kept by the local host in its data folder, protected like `host-token` (readable by this user only). The app asks its local host for them over the local connection. Nothing is stored in the app's own settings file. **(my call: no keychain needed, and later tools can use the same list)**
- If the list from Tailscale cannot be read, Alex can type the computer's full Tailscale name instead.
- The setting decides who can be reached, not who can reach out. For the Mac to open the PC's sessions, only the PC needs it on. For both directions, both need it on. Pairing itself needs it on at the computer that shows the code.
- Any program that holds the local `host-token` can read the tokens for the other computers. That is accepted: the local token already is the whole computer.

### The host says who it is

`hello_ok` also carries: a stable id for this host (made once, kept in the data folder), the computer's name, the system (Mac or Windows) and the home folder. The protocol version goes up by one. A `hello` with a different protocol version gets an answer that names both versions, before the token is looked at, so the app can say which computer needs updating instead of failing silently.

The checks on where a web page connection may come from stay as they are on both listeners.

### The app holds one connection per computer

- The app connects to its local host as today, then to each paired computer, straight from the app to that computer's host. Nothing is relayed through the local host. **(my call, and what `DESIGN.md` already says: "clients attach to any host")**
- Other computers are addressed by their full Tailscale name (`<computer>.<tailnet>.ts.net`), and the app's security policy (both the release one and the development one) is widened only to `ws://*.ts.net:47211`, the host's standard port. **(my call: the policy cannot express "any `100.x` address", and allowing every address or every port would weaken it)**
- Everything the app keys by tab, group, pane or session becomes keyed by computer plus that id. "Computer" here is the entry in this computer's own list of paired computers, never an id or name the other host reports. What the other host reports is shown, not trusted as a key.
- The app treats another computer's host as untrusted input. Every message from it is checked before use: known message types only, known values for states and kinds, length limits on names, titles, folders and tree labels. Nothing a host sends is ever placed into the page as HTML, only as text. Today's app does neither (messages are used as they arrive, and the session state goes into HTML), which is acceptable for the local host and not for another computer's. **(my call, from the security check: otherwise a broken-into PC could run script inside the Mac's app, and the app can start shells on the Mac)**
- Each computer has its own state: connecting, connected, offline. An offline computer is retried in the background with the same backoff the local host has today. Its trouble never blocks or delays the local computer.
- The window closes only when Alex closes the last tab across all connected computers. A computer going offline never closes the window.
- Cmd+T opens the new tab on the computer the current tab is on, in that tab's folder. With no tab focused it opens locally. **(Alex asked for new tabs on the other computer; the rule is from `DESIGN.md`)**

### The sidebar

The look is not decided here. It goes through a review page with a real screenshot and a working mockup before any sidebar code is written. What it has to do:

- Every row shows which computer it lives on, without adding a forced layer of grouping. **(Alex: no forced grouping)**
- Each computer keeps its own tree on its own host. Showing one project that exists on both computers as one group, if the review picks that, is done in the app's display only. No host learns about another host's tree.
- A way to show only one computer's sessions. **(Alex, from the Orca notes: a filter by device)**
- A plus button to start a terminal on a given computer, which also works when that computer has no sessions yet.
- An offline computer shows as offline. Its last known rows stay visible, dimmed, while the app stays open.
- Settings gets the switch, "Pair a computer", "Add a computer", and the list of paired computers with a remove button.

### One size per terminal

`DESIGN.md` already has the rule: a terminal has one size, set by the device Alex last typed in, and other devices never resize it.

- Each session has one owner: one connection whose size is applied and whose input is accepted.
- A view becomes the owner by an explicit claim message, which the app sends only when Alex really presses a key, pastes or clicks in that terminal, right before the input itself. The host does not guess the owner from input arriving. **(my call, from the security check: a terminal view also sends automatic answers to questions the program asks, through the same path as typing, so "whoever sent input last" would let a view Alex is not touching steal the size)**
- The host ignores input for a session from any connection that is not its owner. So a view that is only showing a terminal sends nothing, and only the owner's view answers the program's automatic questions.
- Another connection that attaches or resizes does not change the terminal's size. It is told the real size and draws the terminal at that size: blank space if its pane is bigger, cut off at the right and bottom if smaller. **(my call: no scaling in this work)**
- When a view claims a session, the terminal takes that view's size. The other views are told and redraw.
- When the owner disconnects, the remaining view that attached last becomes the owner and the terminal takes its size, so nobody is left looking at a cut-off terminal they cannot fix. With no view left, the size stays as it is. A connection that attaches is always told the current size, and the first view of a session with no owner becomes its owner.
- With one computer attached, nothing changes from today.

### A slow or sleeping computer never freezes the others

Today a client that falls more than 4 MiB behind makes the host hold the program back for everyone, and the host has no way to notice a connection whose other end has gone silent. With a second computer attached, a closed laptop lid would freeze the terminal on the computer Alex is using.

- Only the owner's connection can hold a program back. Any other connection that falls behind is taken off the live stream and sent a fresh picture of the screen when it has caught up.
- The host pings every remote connection and drops one that stops answering. **(my call on the numbers: a ping every 10 seconds, dropped after 30 seconds of silence; the plan may tune them)**

### Staying up on the PC

- `term-host service install` gets a Windows version that starts the host when Alex logs in, without needing administrator rights. Restart after a crash is wanted. If the chosen Windows mechanism cannot do that, the plan says so plainly instead of promising it.
- Starting before anyone has logged in is not part of this. `DESIGN.md` already covers that gap with Windows automatic login.
- The app has never been run by hand on Windows. Running it there and checking it is part of this work, not an assumption.

## What carries over to the native app

- Everything in the host: the listener, tokens, pairing, identity, the size rule, the Windows start at login.
- The sidebar and Settings, which stay web.
- Not the app's per-computer connection code in the web page. The native app will hold connections in Rust, so that part is written again. It is kept as small as it can be for that reason.

## Risks

- A paired computer can start shells on this one. That is the feature, and it is why pairing is by hand, tokens are per computer and can be removed, and the setting is off until Alex turns it on.
- Any device on the tailnet can reach the listener and try a token or a code. The limits above make guessing hopeless, a token only works from the address it was paired from, and pairing only works from Alex's own devices. Alex can also restrict the port to their own devices in Tailscale's access rules; that is optional hardening and is not needed for a tailnet that holds only Alex's devices.
- A paired computer that is broken into is this computer broken into. A shell it starts here runs as Alex, and can read this host's data folder: the local `host-token` and the tokens for reaching other computers. So the "local only" messages do not hold against a hostile paired computer, and removing it afterwards does not undo what it may have copied. Changing the local token and re-pairing the others after a removal is not built in this work. **(my call: say it plainly instead of building a fix for a case where the attacker already has a shell)**
- A paired computer's host could also send bad data to this computer's app. The checks in "The app holds one connection per computer" are what stand against that.
- The host depends on Tailscale's command line tool being where it expects. If it cannot be found, the setting shows an error naming the places it looked.
- The launchd service on the Mac may not be allowed to run the App Store build's tool. It works from a shell today. The plan proves it from the service first, before building on it.
- The JSON printed by `tailscale status --json` may change between versions. Only the list in "Add a computer" depends on it, and typing the name is the fallback.

## Testing

- Host tests run two throwaway hosts on one computer. The "Tailscale address" and the allowed source range can be set by tests, so no test runs Tailscale's tool, listens on a real Tailscale address, or touches port 47211 or the real data folder.
- Tests cover: the local token refused on the remote listener; a token refused from an address other than the one it was paired from; a removed computer cut off at once, including a connection racing the removal; a code refused after expiry, after use and after 5 wrong tries; pairing refused from a source that is not one of the user's own devices, and with the host's own id; no reverse token handed out while the switch is off; local-only messages refused from a remote connection; a source outside the allowed range refused before the handshake; an oversized first message refused; the try limit; the listener closing when the tailnet changes; pairing giving both directions; the size rule with two connections, including automatic answers from a view that never claimed, and the owner disconnecting; a stalled second connection not holding the program back for the owner; a silent remote connection dropped by the ping.
- App tests cover keys that hold a computer and an id, one computer offline while the other works, the window close rule, and a hostile host: markup in a state, title, folder and tree name shows as plain text, and unknown or oversized messages are dropped.
- Hand checks, by Alex, on both computers: pair, see the other computer's sessions, open and type, new tab on the other computer, resize on one while the other shows the same terminal, unplug one computer, remove a paired computer, restart the PC and find its sessions reachable after login.

## Left to the builder

- The names of the new messages and files, the exact limits, the hash, how the host finds Tailscale's tool on Windows.
- The Windows start at login mechanism.
- How the app is built for the PC (built on the PC from the repository, or taken from a CI build).
- The open-file limit the Mac's service runs with, now that the host takes connections from more than one app.
- The order of the work, as long as two things hold. The unproven parts are proven first: the Mac's service running Tailscale's tool, the Windows firewall, and building the app on the PC. And the host's security parts land and are tested before the app can reach another computer.
