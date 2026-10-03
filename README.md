<p align="center">
  <img src="assets/vyber.png" alt="Vyber icon" width="128" height="128">
</p>

<h1 align="center">Vyber</h1>

<p align="center">
  A native, GPU-rendered terminal workspace with a file tree, previews, Git review and source control.<br>
  Built in Rust on <a href="https://github.com/longbridge/gpui-kit">GPUI Kit</a> and the
  <a href="https://github.com/alacritty/alacritty">Alacritty</a> terminal core — no webview, no Electron.
</p>

<p align="center">
  <img src="assets/screenshot.png" alt="Vyber with a split terminal and the file panel open" width="900">
</p>

> **Release:** 0.1.0. GitHub CI builds Windows x64, macOS Apple Silicon/Intel and Linux x64/ARM64.
> Desktop acceptance is separate from CI: Windows 11 is the development platform; macOS/Linux
> desktop and GPU compatibility are being verified. Packages are unsigned on Windows
> and ad-hoc-signed (not notarized) on macOS.

See [Releasing Vyber](docs/RELEASING.md) for the five workflows/configuration files, packaging,
download verification and the manual publication gate. Releases are prepared as drafts.

## Features

**Terminal**

- Real PTY sessions (ConPTY on Windows) rendered with `alacritty_terminal`: true color, 256 colors,
  alternate screen, bracketed paste, OSC 8 hyperlinks and TUI programs.
- Shift+Enter preserves its modifier for multiline input in Codex and Claude Code, with Kitty
  keyboard protocol negotiation and native Windows console input. Ctrl+T reaches the terminal
  application; use Ctrl+Shift+T to open a Vyber tab on Windows.
- Tabs are stable terminal groups: selecting a group never expands or shifts the tab strip.
  Double-click to name a group, right-click to rename/split/close it, or use the arrow beside `+`
  to pick from all open groups. Group names and the last focused terminal are remembered.
- Split terminals have their own compact, draggable titles with focus/restore and close buttons;
  a lone terminal needs no extra header. Drag group tabs to reorder, drag a terminal title onto
  another pane's edge to move it, or drop it next to `+` to give it its own tab. The strip scrolls
  at its edges during a drag. Shell processes survive every layout change. Closing a multi-terminal
  group asks for confirmation; a pane's close button closes only that terminal.
- Ctrl/Cmd+click opens local paths such as `src/main.rs:42` in the file panel and URLs in the browser.
- On Windows the default shell is **Git Bash** (found via `PATH` or the standard install locations),
  falling back to PowerShell. Your shell profiles are never modified.
- Scrollback search, a persistent font size, and desktop notifications when a terminal rings the
  bell or sends a notification while Vyber is in the background.
- Box-drawing, block and Powerline characters are drawn to fill their cells, so boxes, rules and
  block logos join without gaps whatever the font.

**Files and previews**

- A file panel that slides in over the active terminal without resizing it, or docks beside it and
  moves the terminal aside (the button left of the sidebar toggle switches). Every terminal keeps
  its own tree, open files and preview. Drag the panel's left edge to resize it, drag or toggle the
  sidebar the same way; sizes are remembered.
- The zoom shortcuts size whatever has focus: the terminals, the file panel or source control, each
  on its own and saved in `config.toml`.
- Workspace tree with Git status colors, fuzzy file filtering and content search via
  [ripgrep](https://github.com/BurntSushi/ripgrep). Nested Git repositories can be picked as the tree root.
- Built-in editor with syntax highlighting for Rust, Go, TypeScript/TSX, JavaScript, Python, JSON,
  TOML, Markdown, Bash, CSS and HTML; search/replace, undo/redo, line numbers and atomic saves.
- Rendered Markdown preview, image preview (PNG, JPEG, WebP, GIF, BMP, SVG, ICO) with zoom, and an
  info card for binary or oversized files.
- **Follow** mode opens files as they change on disk; **Pin** docks the preview beside the terminal.

**Review**

- **Turn badge** — while Claude Code or Codex works in a terminal, a small `6 files changed +551 −3`
  badge sits just above its input box and updates live. An active turn stays visible through TUI
  redraws and long task lists: when the input cannot be recognized it keeps its last verified
  position, or uses the terminal's top-right corner (also while viewing scrollback). Only explicitly
  dismissing it hides an active turn. Vyber draws it on top of the terminal: the agent never sees it
  and the terminal size doesn't change. Click it to review that turn. Narrow panes keep the file
  count visible and put the full addition/deletion totals in the tooltip.
- **One scrolling diff** — every changed file in a single list with syntax highlighting, unchanged
  runs folded into "N unmodified lines", split or unified view, line wrapping and find. The changed
  files tree beside it shows added, modified and deleted files and jumps to each one.
- **Sources** — the last agent turn (or an earlier one), uncommitted, unstaged or staged changes, any
  recent commit, or the whole branch against its merge base with the default branch.
- **Revert** a hunk, a file or everything a turn changed; staged and committed views are read-only.
  **Comment** on a line to type `path:line — note` into the terminal's input without sending it.
- Agent turns come from the local Claude Code / Codex session logs (read-only), which mark where a turn
  started and ended. Manual checkpoints work with any command.
- A turn's badge goes to the terminal the agent runs in: Claude Code names each session's process,
  and a Codex session goes to the terminal running `codex`. Turns from the Codex app or an editor
  extension show only in Review.
- An agent started in a folder that holds repositories without being one, such as `Documents`, gets
  a snapshot of the repositories below it. Linked worktrees, such as the ones agents check out under
  `.claude/worktrees`, are left out of every snapshot.
- Snapshots live in a separate object store in Vyber's data directory: your index, refs, stash and
  `.git/objects` are never touched. Files changed after a snapshot are never overwritten silently; a
  recovery copy is kept first.

**Projects**

- A project is a named set of source folders, the first one primary, like a Codex project
  (≡ ▸ Edit project…). Opening a folder that holds repositories offers them as a project, and a
  project already defined in the Codex app is picked up.
- Project folders show in the file tree even when the primary folder's `.gitignore` hides them, as
  in a playground that keeps its repositories side by side. Worktrees can be picked as the tree root.
- An agent turn in the primary folder snapshots every project repository inside it, so the turn
  badge and Review show changes across repositories.
- Projects live in Vyber's data directory; nothing is written into the folders.

**Source control**

- The branch button left of the file panel button (Ctrl+Shift+G) opens source control laid out like
  Cursor's: every repository of the project and its worktrees, a commit box at the top, Merge,
  Staged and Changes groups, and the commit graph with branch lanes, refs and the commits to pull
  or push.
- Stage, unstage and discard files or everything, and single hunks from the diff. Commit, amend,
  commit and push or sync, undo the last commit; fetch, pull (with rebase), push, force push with
  lease, publish a branch.
- Switch branches from a picker; when local changes are in the way, stash them, carry them over or
  discard them. Create, rename, delete, merge and rebase branches, check out remote branches, tags
  or commits; cherry-pick, revert and reset commits; stashes, tags, remotes and worktrees; continue
  or abort merges and rebases and take either side of a conflict.
- Git runs as the command line with your configuration, hooks and credential helpers, and the Git
  Output view lists every command. Background fetches never prompt for credentials; discards and
  hard resets keep recovery copies; a running agent turn is pointed out before commands that
  change its files.

Vyber never talks to agents: it doesn't install hooks or use an SDK. It checks local process names
only when confirming terminal closure. `claude`, `codex`
and every other CLI simply run in your terminal.

## Building

Requirements: a recent stable Rust toolchain (edition 2024) and Git. ripgrep (`rg`) is optional and
only needed for content search.

```sh
cargo build --release --locked
./target/release/vyber            # or: vyber <folder>
```

A fresh launch without a folder opens your home directory. With `restore_workspace` enabled,
previous tabs and folders are restored instead. Tab labels show the folder name; on macOS,
the default zsh session reports directory changes without modifying your shell profiles.

On Windows, `scripts/build.ps1` builds, copies the binary to `dist/Vyber.exe` and creates a
`Vyber.lnk` shortcut with the app icon:

```powershell
.\scripts\build.ps1            # fast development profile
.\scripts\build.ps1 -Release   # optimized build
```

On macOS (with the Xcode command line tools installed), `scripts/build-macos.sh` produces an
ad-hoc-signed `.app` bundle; pass `--install` to copy it to `/Applications`. This path is untested.

Run the test suite with `cargo test --locked`.

## Keyboard shortcuts

| Action | Windows | macOS | Linux |
| --- | --- | --- | --- |
| New tab | Ctrl+Shift+T | Cmd+T / Cmd+N | Ctrl+Shift+T |
| Split right (terminal focused) | Ctrl+D | Cmd+D | Ctrl+Shift+D |
| Split down | Ctrl+Shift+D / Ctrl+Shift+E | Cmd+Shift+D | Ctrl+Shift+E |
| Close active preview file, then panel, then focused terminal | Ctrl+Shift+W | Cmd+W | Ctrl+Shift+W |
| Quit (confirm running terminal processes) | Alt+F4 | Cmd+Q | Ctrl+Shift+Q |
| Next terminal | Ctrl+Shift+] | Cmd+] | Ctrl+Shift+] |
| Switch tab | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Go to tab 1–8 / last tab | Ctrl+1…8 / Ctrl+9 | Cmd+1…8 / Cmd+9 | Ctrl+1…8 / Ctrl+9 |
| Maximize terminal | Ctrl+Shift+Enter | Cmd+Enter | Ctrl+Shift+Enter |
| File panel | Ctrl+Shift+B | Cmd+B | Ctrl+Shift+B |
| Source control | Ctrl+Shift+G | Cmd+Shift+G | Ctrl+Shift+G |
| Find file | Ctrl+Shift+P | Cmd+P | Ctrl+Shift+P |
| Search terminal history | Ctrl+Shift+F | Cmd+F | Ctrl+Shift+F |
| Open folder | Ctrl+Shift+O | Cmd+O | Ctrl+Shift+O |
| Manual checkpoint | Ctrl+Shift+K | Cmd+Shift+K | Ctrl+Shift+K |
| Save in editor | Ctrl+S | Cmd+S | Ctrl+S |
| Close the file in the panel (the panel when no file is open) | Ctrl+W | Cmd+W | Ctrl+W |
| Next / previous panel tab | Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+PgDn / Ctrl+PgUp | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+PgDn / Ctrl+PgUp |
| Find in the Review diff | Ctrl+F | Cmd+F | Ctrl+F |
| Copy / paste in terminal | Ctrl+Shift+C / V | Cmd+C / V | Ctrl+Shift+C / V |
| Settings file | Ctrl+Shift+, | Cmd+, | Ctrl+Shift+, |
| Shortcut help | Ctrl+Shift+H | Cmd+Shift+H | Ctrl+Shift+H |
| Font size of the focused terminal or panel: up / down / reset | Ctrl++ / Ctrl+- / Ctrl+0 | Cmd++ / Cmd+- / Cmd+0 | Ctrl++ / Ctrl+- / Ctrl+0 |

Cmd+W closes the displayed file first, keeping the preview panel open. With no file displayed it
closes the preview or Git panel; with the panel closed it closes only the focused terminal,
including in a split. The other terminals in that split keep running. The terminal close button
has the same scope. Closing a complete group is an explicit action and checks every terminal in it.
Unsaved editor changes block file and terminal closure. Running agent sessions and background jobs
require confirmation; Cancel leaves the sessions running. Cmd+Q checks all open terminals before
quitting and saves the open layout and drafts for restoration.

Closing the last terminal leaves an empty workspace where Cmd+T or the New terminal button opens
a new session. Exiting the last shell has the same behavior. On macOS, closing the window keeps the
application running; launching it again from the Dock or Finder opens a workspace window. New tab
and Quit also work from the native menu when no window is open. Closed terminals are removed from
the saved workspace. The Unix process check reads process names, without command arguments; if
inspection is unavailable, closing a live terminal requires confirmation.

In the file tree: ↑/↓ to move, →/← to expand or collapse, Enter to open, Space for quick look.
In the filter box, Enter opens the best match and Shift+Enter searches file contents.

## Configuration and data

Vyber stores its data in `%LOCALAPPDATA%\Vyber` on Windows and in the user application data
directory on macOS/Linux (`$XDG_DATA_HOME/Vyber` or `~/.local/share/Vyber` on Linux).

| File | Contents |
| --- | --- |
| `config.toml` | `shell`, `font_family`, `font_size`, `files_font_size`, `git_font_size`, `panel_mode` (`"overlay"` or `"dock"`), `scrollback`, `reduced_motion`, `notifications`, `restore_workspace`, `task_history_days`, `task_history_limit`, `git_autofetch`. Changes apply as soon as the file is saved. |
| `workspace.json` | Tabs, split layout, terminal folders, open files and unsaved drafts |
| `projects.json` | Projects and their source folders |
| `git-drafts.json` | Unsent commit messages |
| `checkpoints/`, `tasks/`, `recovery/` | Local review snapshots and recovery copies |
| `vyber.log` | Application errors only — terminal output is never logged |

## Known limitations

- The Claude Code / Codex session log formats are not a stable public API; if they change, the Tasks
  view may stay empty until updated. Manual checkpoints always work.
- Text preview is limited to 8 MB and file loading to 32 MB; this is not a streaming editor for huge files.
- The checkpoint object store is not compacted automatically yet, so keep an eye on its size.
- Codex doesn't name its process in its log, so a Codex session goes to the terminal in its folder
  that runs `codex` and where Enter was pressed last when the session's first turn starts. Two Codex
  sessions started in the same folder at the same moment can be mixed up.
- Turns running at the same time in overlapping folders are flagged, since their changes overlap in
  the review.
- Reviews and the turn badge need a Git repository in the folder or below it; a folder of
  repositories is snapshotted up to 20,000 files.
- Source control has no generated commit messages, pull request view, blame or interactive rebase.
- No PDF or web preview and no LSP.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option. Unless you explicitly state otherwise, any contribution you submit for inclusion in
this project shall be dual licensed as above, without any additional terms or conditions.

Bundled file and UI icons come from Seti UI (MIT), Markdown Mark (CC0) and Lucide (ISC/MIT); see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for these and all Rust dependency licenses.
