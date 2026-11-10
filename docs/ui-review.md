# Sidebar review: settled design

Exact values for the sidebar, group headers, state markers and keys, recorded after Alex said go on the review page, 2026-11-10. Source of truth for every look value is the final mock (`mock.html`, line numbers as `(mock N)`).
Review pages: new https://claude.ai/artifact/GJuwmG4PKSc2HZCr5G4iAx (version 5), old and frozen https://claude.ai/artifact/QYH9TCDXyNSeUmqFGCVScY (version 11). Mock params that match the answers: `spin=tick`, `ghead=rule`, `theme=mac`, `rows=compact`, `sep=both`, `markers=a`, `work=blue`, `glint=off`, `devglyph=icon`, `dim=0`.

Glyphs are written as code points where a font could draw them as emoji. Never use an emoji glyph. Motion is transform and opacity only. Never orange.

## Answers

| Question | Answer | Decided by |
|---|---|---|
| rows (old) | compact (two-line also built, see rulings) | Alex |
| markers (old) | set a | Alex |
| row-height (old) | 26 px (terminal theme; Mac theme 28 px) | Alex |
| width (old) | 240 px, drag the edge to resize | Alex |
| ungrouped (old) | top, no header | Alex |
| pane-keys (old) | keep Cmd+Option+arrows | Alex |
| keys (old) | table is fine | Alex |
| device-mark (old) | laptop and desktop icons | Alex |
| separation (old) | both: faint edge line and faint line above each group | Alex |
| handoff-state (old) | its own marker | Alex |
| working-look (new) | tick: 15 degree nudge every 1 s, soft spring settle, 4 s loop | Alex |
| folder-head (new) | rule: one hairline above each group | Alex |
| failed-state (new) | yes, keep a red Failed state (R54) | Claude (Alex left it blank: "no clue") |
| sounds (new) | per state: sound, banner, dock badge or nothing (R56). Later plan, recorded only | Alex (chat) |
| mac-theme (new) | Mac is the default sidebar theme, Terminal is the other (R55) | Alex |

Other rulings that shape the values:

| Ruling | What | By |
|---|---|---|
| R45 | No centered sidebar. Smooth group collapse stays | Alex |
| R50 | No sparkles on the working snowflake (mock `glint=off`) | Alex |
| R53 | Group headers have no arrow. The folder icon sits in the marker column and is the open and closed control. Its front flap opens when the group is expanded, 180 ms, transform only | Alex |
| R24 | Build both compact (default) and two-line rows behind one setting `sidebar_rows: "compact" or "two"`. The cmux three-line design is dropped | Alex |
| R32 | Unfocused panes do not dim by default. Dimming to 58 percent is a setting, off by default. Unfocused panes show a hollow cursor | Alex |
| R31 | Pane focus follows the pointer, also on the sidebar pane row. It stays after the pointer leaves | Alex |
| R33 | Selected row and hover use kitty's selection color #F5E0DC at 16 and 6 percent | Alex (reading is Claude's) |
| R34 | Group arrow was theme foreground. Superseded by the folder icon (R53) | Alex |
| R37 | App is named snowflake. The rename is its own task after this plan | Alex |
| R57 | Markers morph into each other on a state change. Pending, look not final (see "Pending") | Alex |

## Colors

All from `client/src/theme.ts`, copied into the mock `:root` (mock 14-33). Roles and tints below it (mock 35-46, 271, 422).

| Variable | Value | Use |
|---|---|---|
| `--background` | #16141F | terminal and sidebar surface (terminal theme) |
| `--foreground` | #CDD6F4 | strong text, selected name, `--text-strong` |
| `--cursor` | #F5E0DC | caret, hollow cursor outline |
| `--cursor-accent` | #1E1E2E | menu and drag ghost fill, `--sel-fg` |
| `--sel-bg` | #F5E0DC | rename highlight base |
| `--black` | #45475A | `--line`: hairlines, scrollbar thumb, drag ghost border, pane index digit, usage line text |
| `--red` | #F38BA8 | blocked, failed |
| `--green` | #A6E3A1 | done |
| `--yellow` | #F9E2AF | handoff requested |
| `--blue` | #89B4FA | `--accent`: focus ring, drag line, join, working marker |
| `--white` | #BAC2DE | `--text-head` |
| `--bright-black` | #585B70 | `--text-dim`: sub line, branch, count, pane rows, device mark, idle marker |
| `--bright-white` | #A6ADC8 | `--text-name`: row names |
| `--magenta` #F5C2E7, `--cyan` #94E2D5 | | kept for helpers and other sessions, unused in the sidebar |
| `--hover` | rgba(245, 224, 220, 0.06) | row hover (terminal theme) |
| `--selected` | rgba(245, 224, 220, 0.16) | selected row, hot menu item (terminal theme) |
| `--join` | rgba(137, 180, 250, 0.10) | drag-to-join fill |
| `--sep-line` | rgba(205, 214, 244, 0.07) | edge line and group line (mock 271) |
| `--gh-rule` | rgba(205, 214, 244, 0.11) | hairline above a group (mock 422) |
| `--gh-fold` | #7C87AA | folder icon (mock 422) |
| `--mac-sel` | rgba(205, 214, 244, 0.16) | Mac theme selected (mock 379) |
| `--mac-hover` | rgba(205, 214, 244, 0.06) | Mac theme hover |
| `--mac-hair` | rgba(205, 214, 244, 0.08) | Mac theme edge line, menu border |
| `--mac-pop` | #24222F | Mac theme menu and ghost fill |

## Sidebar sizes

| Item | Value |
|---|---|
| Width default | 240 px (`--sb-w`, mock 49) (Claude's call) |
| Width minimum | 168 px (`MIN_WIDTH`, mock 690; fits the 22 px slot plus about 15 characters; 40 tabs must still show every marker) (Claude's call) |
| Width maximum | 480 px (Claude's call; the mock only clamps its URL param to 120 to 600, mock 915, which is not a design value) |
| Drag behavior | drag the sidebar's right edge to resize, saved as `sidebarWidth` (plan Task 5). The handle is a 6 px wide invisible hit area on the sidebar's right edge with `cursor: col-resize`, no snapping, width saved in settings (Claude's call) |
| Top gap | `#sidebar` padding-top 10 px, the window drag strip (mock 65) |
| Marker slot | 22 px wide (`--slot`), as tall as the row, never shrinks (mock 53, 84) |
| Row name indent | `.body` margin-left 4 px after the slot (mock 90) |
| Row padding | 0 `--pad-r` 0 `--pad-l`: terminal 0 10 px 0 8 px, Mac 0 16 px 0 10 px (mock 54, 55, 385) |
| List bottom padding | 8 px; scrollbar 6 px wide, thumb `--line`, radius 3 px, no sticky headers (mock 67-70) |
| Sizes of text | name 13 px (`--sb-font`), sub line and pane rows 12 px (`--sb-small`), count, usage, join tag, meta 11 px (mock 50, 51, 93). Never scales with terminal zoom |
| Font | terminal theme: `"JetBrainsMono Nerd Font Mono", monospace` (mock 30). Mac theme: `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", sans-serif` (`--sys`, mock 377). Markers, folder icon and device glyphs always stay mono |

Row heights (derived by adding the CSS terms, not measured in a browser):

| Row | Terminal theme | Mac theme |
|---|---|---|
| Compact, no second line | 26 px (`--row-h`) | 28 px |
| Compact, blocked or done or failed with second line | 44 px (name 26, sub margin-top -4, sub 16, padding-bottom 6) | 46 px |
| Two-line, every row | 44 px (name 22 with margin-top 2, sub margin-top -2, sub 16, padding-bottom 6) | 44 px, with the slot kept at 26 px (mock 98) |
| Group header | 26 px | 28 px |
| Pane row | 22 px | 22 px |
| Device picker | 30 px | 30 px |
| Menu item | 26 px | 26 px |
| Usage line | 28 px | 28 px |

Row design details (mock 74-105):
- Compact (default): second line (`.sub`, 12 px, `--text-dim`, one line, ellipsis) only on blocked, done and failed rows, with the last title or note.
- Two-line (`sidebar_rows: "two"`): every row has a second line: note if urgent, else the title, else the folder path. Name box 22 px high, `margin-top: 2px` (mock 96-98).
- Name: one line, ellipsis, `--text-name`; selected `--text-strong`.
- Unnamed rows show name, else terminal title, else folder name (plan Task 6).

Spacing between groups (terminal theme, `sep=both`, `ghead=rule`):
- Rule replaces the group line. Above every group except the first element: `margin-top: 8px; padding-top: 4px`, hairline at the top of the section (mock 447-448). Without the rule variant the default is `margin-top: 10px` (mock 72), or 5 px plus the line plus 5 px with `sep-groups` (mock 274-277); not used with the rule.
- Ungrouped tabs sit first, in `.loose`, with no header.

## Group header (hairline variant, `ghead=rule`)

| Part | Value |
|---|---|
| Container `.ghead` | flex, `align-items: center`, `height: var(--row-h)`, `padding: 0 var(--pad-r) 0 var(--pad-l)`, `cursor: default`, `user-select: none` (mock 74) |
| Hairline | `body.ghead-rule #list > .group + .group::before` and `.loose + .group::before`: `content: ""; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--gh-rule)` (mock 447-448). Edge to edge. Never above the first element |
| Hover | `.ghead:hover { background: var(--hover) }` (mock 437). Terminal theme only; the Mac theme keeps this same full-width fill (no Mac override for headers found in the mock) |
| Focus | keyboard focus on the folder button outlines the whole header: `outline: 1px solid var(--accent); outline-offset: -1px` (mock 439-440) |
| Name `.gname` | `flex: 0 1 auto; margin-left: 4px; color: var(--text-strong); font-weight: 600`, ellipsis (mock 432). Terminal theme 13 px. Mac theme 12 px, 600, `--text-strong` (mock 434) |
| Branch `.branch` | `flex: 0 100000 auto` (mock 433), `margin-left: 1.2ch`, `--text-dim`, ellipsis. Mac theme 11 px, `margin-left: 6px` (mock 406). The mock comment says the name shrinks first; the CSS makes the branch shrink first. Follow the CSS and confirm at 168 px |
| Count `.gcount` | `margin-left: auto; padding-left: 8px`, mono, 11 px, `--text-dim`, no wrap. Shows the number of sessions (mock 430) |
| Roll-up `.roll` | after the count, `margin-left: 0`, `width: calc(var(--slot) + 6px)` = 28 px, `padding-left: 6px`, centered. Holds a blocked marker when the group is collapsed and any session in it is blocked (mock 81, 431, 1099) |
| Arrow | none. Group `plain` mode keeps an arrow in the mock but is not chosen |

Folder icon (`FOLDER_SVG`, mock 699 and 423-429):

```
<svg class="gfold" viewBox="0 0 13 13" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round">
  <path d="M1.5 10V3.2C1.5 2.8 1.8 2.5 2.2 2.5H4.9L6.1 3.8H10.8C11.2 3.8 11.5 4.1 11.5 4.5V10C11.5 10.4 11.2 10.7 10.8 10.7H2.2C1.8 10.7 1.5 10.4 1.5 10Z"/>
  <path class="ff" fill="currentColor" fill-opacity="0.22" d="M1.5 5.4H11.5V10C11.5 10.4 11.2 10.7 10.8 10.7H2.2C1.8 10.7 1.5 10.4 1.5 10Z"/>
</svg>
```

| Item | Value |
|---|---|
| Size | 13 by 13 px, `display: block; flex: none; overflow: visible`, centered in the 22 px marker slot, so folder names and session names share one left edge |
| Color | `--gh-fold` #7C87AA; `.ghead:hover .gfold` goes `--text-strong` (#CDD6F4) |
| Flap, group expanded | `.gfold .ff { transform-box: fill-box; transform-origin: 50% 100%; transform: skewX(-22deg) scaleY(0.7) }` (mock 426) |
| Flap, group collapsed | `.collapsed .gfold .ff { transform: none }` (mock 427): a flat, closed folder |
| Timing | `transition: transform 180ms cubic-bezier(0.2, 0, 0, 1)`, the same as the row slide |
| Reduced motion | `.gfold .ff { transition: none }` (mock 429) |
| Control | the 22 px slot is a `<button>` with `aria-expanded` and the label "Collapse name" or "Expand name" (mock 1111) |

## Collapse motion (rows slide, FLIP)

Only on a user click on a folder (mock 520-524, 1364-1470). Transform and opacity only.

| Step | Value |
|---|---|
| Duration and easing | `FLIP_MS = 180`; `transition: transform 180ms cubic-bezier(0.2, 0, 0, 1), opacity 180ms cubic-bezier(0.2, 0, 0, 1)` on `.flip-run` (mock 522) |
| First | measure every visible header, row, pane row and the drag insert line; also copy the rows of a group being closed |
| Last | render the new list, measure again |
| Invert | each moved element gets `translateY(old top - new top)` with no transition. Rows that enter ride with their header: `translateY(-headerShift - 4px)` and `opacity: 0` |
| Play | add `.flip-run`, clear the transform and opacity |
| Closing | copies of the closing rows sit in a `.flip-out` layer clipped to the list (`position: absolute; overflow: hidden; pointer-events: none; z-index: 1`), fade to opacity 0 and move with the header (`translateY(headerShift)`) |
| Folder flap | animates from its old transform with the same timing |
| Cleanup | after `FLIP_MS + 80` ms, and before every render, so an interrupted run never leaves a transform behind |
| Reduced motion | skip the whole move and change instantly (mock 1368, 1402) |

## State markers (set a)

Markers sit in the 22 px slot. `.mk` is `inline-flex`, centered, `line-height: 1`, `font-style: normal` (mock 116). Set a: `font-size: 15px; width: 14px` (mock 118). Every marker has `role="img"`, an `aria-label` and a `title` (labels at mock 701-702). Sets b and c are not chosen and are not recorded here.

This plan builds working, blocked, done and idle; failed and handoff requested are recorded looks for a later plan (R58).

| State | Shape | Size | Color | Motion |
|---|---|---|---|---|
| Working | drawn snowflake, six arms (SVG below) | 13 by 13 px SVG in a 13 px `.fspin` wrapper, `margin-right: 1px`, stroke 1.4 | `--blue` #89B4FA, full brightness (`.work-loud`, mock 173) | the tick turn below |
| Blocked | glyph U+25C6 (black diamond) | `font-size: 16px` (mock 120) | `--red` #F38BA8 | none. Still. Loudest marker |
| Done | glyph U+2713 (check mark) | 15 px, bold (mock 121) | `--green` #A6E3A1 | none |
| Idle | rounded diamond: a `span.dia`, 7 by 7 px, `border-radius: 2px`, `background: currentColor`, `transform: rotate(45deg)` (mock 149) | 7 px | `--bright-black` #585B70 | none |
| Handoff requested | glyph U+21A9 (leftwards arrow with hook), bold | 14 px (mock 136) | `--yellow` #F9E2AF, never the blocked red | none |
| Failed | glyph U+00D7 (multiplication sign), bold | 15 px (mock 140) | `--red` #F38BA8, quieter than blocked and never the blocked shape | none |
| Rolled up on a collapsed group | the Blocked marker, in `.roll` at the right of the header | as Blocked | `--red` | none. Only blocked rolls up (mock 1099) |

Working snowflake SVG (mock 694-696):

```
<svg class="flake-mk" width="13" height="13" viewBox="-8 -8 16 16" fill="none" stroke="currentColor"
     stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
  six times, a = 0, 60, 120, 180, 240, 300:
  <g transform="rotate(a)"><g class="arm" style="--i:i"><path d="M0 0V-6.5M0-4.1L-1.9-5.9M0-4.1L1.9-5.9"/></g></g>
</svg>
```

`.flake-mk` has `overflow: visible`; `.flake-mk .arm` has `transform-box: view-box; transform-origin: 0 0` (mock 150-151). The `margin-right: 1px` on the flake puts its vertical arm on a half pixel so it stays crisp.

Working motion, tick (`spin-tick`, mock 513-518): `.mk.a .fspin { animation: fl-tick 4s linear infinite; will-change: transform; transform-origin: 50% 50% }`. The marker, the slot and the row never move; the star repeats every 60 degrees so the loop is seamless. `fl-tick` is four equal steps of 15 degrees (mock 458-512). Quarter 1 is written in absolute degrees; quarters 2 to 4 repeat it with 15, 30 and 45 added (0%+25%, 50%, 75% blocks):

| Keyframe % in quarter 1 | rotate (deg) | | Keyframe % in quarter 1 | rotate (deg) |
|---|---|---|---|---|
| 0 | 0.00 | | 7.5 | 16.37 |
| 1 | 1.86 | | 9 | 15.88 |
| 2 | 5.73 | | 10.75 | 15.26 |
| 3 | 9.77 | | 12.75 | 14.91 |
| 4 | 13.00 | | 15 | 14.88 |
| 5 | 15.09 | | 17.5 | 15.00 |
| 6.25 | 16.27 | | 25 | 15.00 (hold) |

Absolute stops for the other quarters: 26/27/28/29/30/31.25/32.5/34/35.75/37.75/40/42.5/50 give 16.86/20.73/24.77/28.00/30.09/31.27/31.37/30.88/30.26/29.91/29.88/30.00/30.00; 51/52/53/54/55/56.25/57.5/59/60.75/62.75/65/67.5/75 give 31.86/35.73/39.77/43.00/45.09/46.27/46.37/45.88/45.26/44.91/44.88/45.00/45.00; 76/77/78/79/80/81.25/82.5/84/85.75/87.75/90/92.5/100 give 46.86/50.73/54.77/58.00/60.09/61.27/61.37/60.88/60.26/59.91/59.88/60.00/60.00. So: a nudge of 15 degrees in the first 5 percent of each quarter (about 0.2 s), a peak of about 16.4 degrees (a soft spring overshoot of about 9 percent), settled by 17.5 percent, then a rest until the next second.
- Window not focused: `animation-play-state: paused` (mock 515), so 40 tabs do not animate behind other apps.
- `prefers-reduced-motion: reduce`: `animation: none` (mock 516-518).
- No sparkles, no sheen, no scale, no fade on the working marker (glint is off, R50). Do not build the `spk` and `shn` code.
- The mock's script derives the start offset from the clock with `SPIN_MS.tick = 7200` (mock 718), left over from an older 1.2 s step. The built value is a 4000 ms loop, so the modulus is 4000.

## Selection, hover, focus, rename

| Item | Terminal theme | Mac theme |
|---|---|---|
| Hover | `.row:hover` background `--hover` rgba(245, 224, 220, 0.06), full width (mock 87) | `::before` rect inset 8 px from both sidebar edges, `border-radius: 6px`, `--mac-hover` (mock 395-397) |
| Selected | background `--selected` rgba(245, 224, 220, 0.16), name `--text-strong`. No blue block, no left bar (mock 88) | same rect, `--mac-sel` rgba(205, 214, 244, 0.16), no left bar (mock 398) |
| Keyboard focus | `outline: 1px solid var(--accent); outline-offset: -1px` on the row (mock 89) | same outline on the `::before` rect (mock 400) |
| Menu open | `.row.menu-open` background `--hover` (mock 240) | `--mac-hover` (mock 397) |
| Row arrow keys | Up and Down move focus between visible rows, Enter or Space selects (mock 1647-1656) | same |

Rename field (mock 228-233):
- `.rename`: flex, `height: 20px; margin: 3px 0; margin-left: -5px; padding: 0 5px; border-radius: 4px`, `background: rgba(205, 214, 244, 0.06)`, `outline: 1px solid var(--accent); outline-offset: -1px`, text `--text-strong`. The 3 px above and below keep the row at 26 px; the -5 and +5 px keep the name's left edge in place.
- Selected text `.hl`: `height: 16px; line-height: 16px; border-radius: 2px; background: rgba(245, 224, 220, 0.82); color: #1E1E2E` (`--sel-fg`).
- Caret: `width: 1px; height: 14px; margin-left: 1px; background: var(--cursor)`.
- Mac theme: `height: 22px; border-radius: 6px` (mock 409). Over a see-through window the fill is a solid #21202C (mock 325, look only).
- Opens on Cmd+Shift+R or double click, Enter saves, Esc cancels, typing focus returns to the terminal (old page text; plan Task 6). A set name is never overwritten by the terminal title; an empty name brings the automatic name back.

## Focus and pane dimming

| Item | Value |
|---|---|
| Focus rule | pointing at a pane, or at its sidebar pane row, moves typing focus there. It stays after the pointer leaves (mock 1670-1679) |
| Focused pane | full brightness; its sidebar pane row gets `.focused`: name color `--text-name` |
| Unfocused pane | full brightness by default. Hollow cursor: `background: none; color: inherit; outline: 1px solid var(--cursor); outline-offset: -1px` (mock 304) |
| Dim setting `dim` | off by default. When on: `.dim-panes .pane.dim .screen { opacity: .58 }` (mock 303), `transition: opacity 120ms ease-out` (mock 282) |
| Pane corner marker | `position: absolute; top: 2px; right: 2px; width: 22px; height: 20px; font-size: 13px; background: var(--background)`; the same marker as the row (mock 305-308). Inside a split, `right`/`top` offsets are 0 or 8 px per pane (mock 306-308) |
| Terminal inset | `#term` padding 8 px; split dividers 1 px `--line` with 8 px padding on each side of it (mock 293-299) |

## Pane list under a split tab

Shown under the selected split tab (mock 108-113):
- `.panes` padding `0 0 4px 0`.
- `.pane-row`: flex, `align-items: center`, `height: 22px`, `padding: 0 var(--pad-r) 0 calc(var(--pad-l) + var(--slot))` (30 px left in the terminal theme), `font-size: 12px`, color `--text-dim`; focused `--text-name`.
- Its slot is `18px` wide and 22 px high (mock 110); name `margin-left: 4px`, ellipsis.
- Pane index: `.pidx` color `--line` #45475A, `margin-right: 1ch`, then the pane name.
- Hover: `background: var(--hover)` (mock 281); Mac theme: the inset rounded rect.
- One marker per pane, the same shapes as rows.

## Right-click menu

Four items only (mock 1127, 236-240):

| Item | Key hint shown |
|---|---|
| Rename | U+21E7 U+2318 R (shift, command, R) |
| Pin | none |
| Close | U+21E7 U+2318 W (Cmd+Shift+W), matching the keys table. The mock shows Cmd+W; that is a mock error |
| Move to group | U+203A (a submenu arrow) |

Style, terminal theme: `width: 200px; padding: 4px 0; background: #1E1E2E (--cursor-accent); border: 1px solid #45475A; border-radius: 4px; font-size: 13px; z-index: 20`. Item: `height: 26px; padding: 0 10px 0 12px; color: --text-name`; hover or hot `--selected` fill and `--text-strong`; key hint right-aligned, 12 px, `--text-dim`. Position: top is the bottom of the row minus 2 px, left is `min(72, width - 60)` px. Mac theme: `padding: 4px; background: #24222F; border-color: rgba(205, 214, 244, 0.08); border-radius: 6px; font-family: system`, items `padding: 0 8px; border-radius: 4px`, hot `rgba(205, 214, 244, 0.16)` (mock 410-412).

## Drag targets

| Target | Look |
|---|---|
| Drag source row | `opacity: .35` (mock 285) |
| Drag ghost | follows the pointer: `height: var(--row-h); padding: 0 12px 0 4px; background: #1E1E2E; border: 1px solid #45475A; border-radius: 3px; opacity: .96; pointer-events: none; z-index: 15`, holds the slot marker and the name. Mac theme: `#24222F`, border `--mac-hair`, `border-radius: 6px` (mock 290, 415) |
| Between rows (reorder) | an `.insert` line: `height: 2px; background: var(--accent); border-radius: 1px; left: var(--pad-l); right: var(--pad-r)`, centered on the gap (mock 288-289) |
| Onto a row (join as a split) | the row fills `--join` rgba(137, 180, 250, 0.10) with `outline: 1px solid var(--accent); outline-offset: -1px`, plus a "join" tag: 11 px, `--accent`, right edge `--pad-r`, line-height `--row-h` (mock 286-287). Joins on the right by default, below with Shift held (plan Task 6) |
| Pane row out to a gap | pulls the pane out into its own tab (`SplitOut`, plan Task 6). A pane row dragged out to a gap between rows shows the same blue reorder line as a tab reorder (Claude's call) |
| Window drag | the empty part of the sidebar drags the window (`data-tauri-drag-region`, plan Task 5) |

## Device picker and device mark (look only)

| Item | Value |
|---|---|
| Picker `#devpick` | borderless, flex, `height: 30px`, `padding: 0 var(--pad-r) 0 var(--pad-l)`, below the 10 px drag strip, outside the scrolling list. Hover or open: `--hover` (Mac: inset rect) (mock 244-247). Hidden with one computer |
| Parts | mark in the 22 px slot (slot 30 px high), name `margin-left: 4px` ellipsis, chevron U+F0140 at the right: 12 px, `--text-dim`, 20 px wide (mock 249, 1082) |
| Menu | same style as the right-click menu, 200 px wide, items All devices, MacBook Pro, PC, Linux box (offline: dim, "offline" hint, no hover fill). Current device gets the hot fill. `left: 3px` (Mac theme 5 px), top 2 px under the picker (mock 250-254, 1131-1141) |
| Mark style | icon (Nerd Font glyphs, mock 852-857): Mac U+F0322, PC U+F01C5, Linux U+F048B, all U+F037A. 15 px in the picker and menu |
| Mark on rows | combined view only: `.dev`, 14 px wide, `height: var(--row-h)`, `margin-left: 6px`, at the right edge after the name, `--text-dim`, glyph 12 px, so the marker slot never moves (mock 266-267) |

## Usage line

`#usage`: `height: 28px; line-height: 28px; display: block; padding: 0 var(--pad-r) 0 calc(var(--pad-l) + var(--slot) + 4px)` (34 px left in the terminal theme), `font-size: 11px; color: var(--line)` (#45475A), no wrap, ellipsis (mock 226). Empty in the app; it only holds its 28 px so the list never shifts. The mock's text "Usage line, empty for now" is a placeholder. The numbers are a later plan.

## The two themes side by side

Default theme is Mac. Only the sidebar changes; the terminal stays as it is (mock 374-415).

| Item | Mac (default) | Terminal |
|---|---|---|
| Font | system sans (`--sys`), 13 px, normal | JetBrains Mono Nerd Font Mono, 13 px |
| Row height | 28 px | 26 px |
| Padding left and right | 10 and 16 px | 8 and 10 px |
| Panel fill | `rgba(205, 214, 244, 0.034)` over #16141F (about #1C1A27) | none, the terminal background #16141F |
| Edge line | 1 px at the right, `--mac-hair` rgba(205, 214, 244, 0.08) | 1 px, `--sep-line` rgba(205, 214, 244, 0.07) |
| Hover and selected | rounded rect, inset 8 px each side, radius 6 px, 6 and 16 percent of #CDD6F4 | full-width rect, 6 and 16 percent of #F5E0DC |
| Group name (rule header) | 12 px, 600, `--text-strong` | 13 px, 600, `--text-strong` |
| Branch | 11 px, `margin-left: 6px` | 13 px, `margin-left: 1.2ch` |
| Group hairline | same (`--gh-rule`) | same |
| Rename field | 22 px high, radius 6 px | 20 px high, radius 4 px |
| Menus and drag ghost | #24222F, border `--mac-hair`, radius 6 px, menu padding 4 px, item radius 4 px, system font | #1E1E2E, border #45475A, radius 4 px (ghost 3 px), mono |
| Markers, folder icon, chevrons, device glyphs | mono font, 22 px slot (same) | same |
| Keys | same | same |

Over a see-through window the Mac panel is `rgba(205, 214, 244, 0.06)` (mock 388, look only).

## Keys

Final. Answers: keys = ok, pane-keys = keep Cmd+Option+arrows. This is the table from the old review page and plan Task 6 (old page 936-949), plus the extra keys from `DESIGN.md` "Keys". kitty's own keys stay for everything kitty has; where one here differs, this one wins.

| Action | Key | Note |
|---|---|---|
| New tab | Cmd+T | a shell in the current pane's live folder |
| Close pane | Cmd+W | ends that pane's session |
| Close tab | Cmd+Shift+W | |
| Reopen closed tab | Cmd+Shift+T | a Claude tab resumes its chat |
| Next, previous tab | Cmd+Shift+] and [ | in sidebar order, skipping collapsed groups |
| Jump to group 1 to 9 | Cmd+1 to 9 | lands on the tab you last used there |
| Jump to tab 1 to 9 in the group | Ctrl+1 to 9 | |
| Split right, split down | Cmd+D, Cmd+Shift+D | a new shell beside the focused pane |
| Move between panes | Cmd+Option+arrows | kept (pane-keys). The one Option key in the defaults |
| Fill the tab with one pane | Cmd+Shift+Enter | keeps its kitty meaning |
| Show or hide the sidebar | Cmd+B | |
| Rename | Cmd+Shift+R | |
| Zoom in, out, reset | Cmd+= and Cmd+plus, Cmd+minus, Cmd+0 | every terminal, 8 to 40 px, sidebar stays 13 px |
| Copy, paste | Cmd+C, Cmd+V | as today |
| Jump to the oldest blocked session | Cmd+Shift+U | plan Task 9 |
| Board | Cmd+Shift+B | later plan |
| New tab on the other computer | Cmd+Shift+N | later plan |

## Pending, look not final

Markers morphing into each other on a state change (R57): every marker built from one shared set of strokes, moved by transform and opacity only. Not final. Until Alex has seen it, build this fallback (mock 526-590). Easing `--tx-ease: cubic-bezier(0.2, 0, 0, 1)`, overshoot `--tx-over: cubic-bezier(0.34, 1.56, 0.64, 1)`. The old marker sits in a layer over the new one in the same 22 px slot, each pair plays its move, then the layer and classes are removed. The `--tx-m` Slow multiplier (4) is the demo only.

| Pair | Total | Old marker out | New marker in |
|---|---|---|---|
| Any pair not listed, and anything to handoff | 280 ms | `tx-out` 200 ms: opacity 1 to 0, scale 1 to .8 | `tx-in` 240 ms, delay 40 ms: opacity 0 to 1, scale .8 to 1 |
| Idle to working | 350 ms | `tx-turn-out` 200 ms: rotate 0 to 90 deg, scale 1 to .3, fades | marker `tx-show` 120 ms, delay 60 ms; arms `tx-arm` 180 ms, scale 0 to 1, delay `60 + i * 22` ms per arm |
| Working to done | 320 ms | `tx-fold` 200 ms: rotate 0 to -45 deg, scale 1 to .25, fades | check uncovered left to right: a clip window `tx-reveal` translateX(-100%) to 0 while its content `tx-reveal-in` slides 100% to 0, 220 ms, delay 100 ms |
| Anything to blocked | 340 ms | `tx-snap` 120 ms: scale 1 to .4, fades | `tx-pop` 280 ms, delay 60 ms, `--tx-over`: scale .3 to 1, opacity 1 by 40 percent |
| Blocked to working | 340 ms | `tx-calm-out` 200 ms: scale 1 to .7, fades | `tx-calm-in` 280 ms, delay 60 ms: scale .7 to 1, no overshoot |
| Done to idle | 360 ms | `tx-sink` 240 ms: translateY 0 to 2 px, fades | `tx-show` 280 ms, delay 80 ms: fades up |
| Anything to failed | 350 ms | `tx-collapse` 150 ms: scale 1 to 0, fades | two bars (9 by 2 px, radius 1 px) cross into the x: `tx-bar-a` 140 ms delay 100 ms (rotate 45 deg, scaleX 0 to 1), `tx-bar-b` 140 ms delay 170 ms (rotate -45 deg); then the glyph shows in the last 60 ms (`tx-hold-show` 350 ms linear, 83 percent hold) |
| Second line appears | 320 ms | old line copy `tx-sub-out` 180 ms: translateY 0 to -4 px, fades | `tx-sub-in` 260 ms, delay 60 ms: translateY -4 to 0 px, fades in |

Only a state changed in the running app animates. First render and a tree push render instantly. Reduced motion: no animation.

## Look only, not built in this plan

The plan's "Not in this plan" paragraph: a second computer (device filter, offline rows, project list sync, Cmd+Shift+N, `ta new --on`), accounts, `ta` beyond `hook`, the master agent and `claude master`, sounds and system notifications, the board, the pull request line in a group header, helper and team child rows, the dim "maybe waiting" marker, the usage line's numbers, the settings window, the Windows service and the Windows title bar setting. Mock items that fall under it or have no task:

| Mock item | Status | Mock lines |
|---|---|---|
| Settings window (General, Appearance, Sounds pages; 760 by 540, radius 10, `#1F1D2A`, shadow `0 18px 48px rgba(0,0,0,0.42)`) | later plan | 601-641 |
| Sounds per state: sound, banner, dock badge or nothing; none for working and idle | later plan (R36, R56) | 1275-1362 |
| Device picker, device menu, per-row device marks, the combined view | later plan (needs a second computer) | 242-268, 852-897 |
| Master session row and helper rows (helpers one slot in, a "master" tag that hides them, `+N` when collapsed) | later plan | 215-222, 827-883 |
| Snow backdrop (`bg=still or snow`, layer opacity 5, 8 or 12 percent) | look only, off by default | 317-342 |
| See-through window (`glass`, wallpapers) | look only, off by default | 344-372 |
| Mac-style terminal look | later idea (R55), vault Inbox | none |
| Handoff requested marker | look only: the signal (a `term-host hook` call) is a later addition and must not grow plan Task 8 | 135-138 |
| Failed marker | look only until a hook-signal check shows Claude exposes it (R42). Plan Tasks 8 and 9 build four states. Controller to decide whether Task 9 adds it | 139-145 |
| `cmux` three-line rows | dropped (R24) | 100-105 |
| Marker sets b and c, `work=name` and `work=quiet`, `glint`, `spin=slow` and `spin=off` | not chosen, do not build | 123-213 |
| Transitions demo strip (Play, Slow buttons) | mock only | 592-599 |

## Where DESIGN.md disagrees

DESIGN.md was updated to match on 2026-11-10.
