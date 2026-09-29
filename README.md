<p align="center">
  <img src="assets/vyber.png" alt="Vyber icon" width="128" height="128">
</p>

<h1 align="center">Vyber</h1>

<p align="center">
  A native, GPU-rendered terminal workspace with a file tree, previews and Git review.<br>
  Built in Rust on <a href="https://github.com/longbridge/gpui-kit">GPUI Kit</a> and the
  <a href="https://github.com/alacritty/alacritty">Alacritty</a> terminal core — no webview, no Electron.
</p>

<p align="center">
  <img src="assets/screenshot.png" alt="Vyber with a split terminal and the file panel open" width="900">
</p>

> **Status:** early (0.1). Developed and tested on Windows 11. The macOS build script exists but the
> macOS build has not been verified yet.

## Features

**Terminal**

- Real PTY sessions (ConPTY on Windows) rendered with `alacritty_terminal`: true color, 256 colors,
  alternate screen, bracketed paste, OSC 8 hyperlinks and TUI programs.
- Tabs are terminal groups. Split any terminal left/right/up/down, drag titles to reorder, drag a
  pane onto another pane's edge to move it, or drop it next to `+` to turn it into its own tab.
  Shell processes survive every layout change.
- Ctrl/Cmd+click opens local paths such as `src/main.rs:42` in the file panel and URLs in the browser.
- On Windows the default shell is **Git Bash** (found via `PATH` or the standard install locations),
  falling back to PowerShell. Your shell profiles are never modified.
- Scrollback search, a persistent font size, and desktop notifications when a terminal rings the
  bell or sends a notification while Vyber is in the background.

**Files and previews**

- A file panel that opens over the active terminal without resizing it. Every terminal keeps its
  own tree, open files and preview.
- Workspace tree with Git status colors, fuzzy file filtering and content search via
  [ripgrep](https://github.com/BurntSushi/ripgrep). Nested Git repositories can be picked as the tree root.
- Built-in editor with syntax highlighting for Rust, Go, TypeScript/TSX, JavaScript, Python, JSON,
  TOML, Markdown, Bash, CSS and HTML; search/replace, undo/redo, line numbers and atomic saves.
- Rendered Markdown preview, image preview (PNG, JPEG, WebP, GIF, BMP, SVG, ICO) with zoom, and an
  info card for binary or oversized files.
- **Follow** mode opens files as they change on disk; **Pin** keeps the current file in place.

**Review**

- **Changes** — working tree vs. HEAD, working tree vs. index, and index vs. HEAD, in split or unified
  diff view. Reverting only touches the working tree; your index is left alone.
- **Tasks** — reads the local Claude Code / Codex session logs (read-only) to find where an agent task
  started and ended, and shows exactly what changed in between. Manual checkpoints work with any command.
- Snapshots live in a separate object store in Vyber's data directory: your index, refs, stash and
  `.git/objects` are never touched. Files changed after a snapshot are never overwritten silently; a
  recovery copy is kept first.

Vyber never talks to agents: it doesn't install hooks, use an SDK or scan processes. `claude`, `codex`
and every other CLI simply run in your terminal.

## Building

Requirements: a recent stable Rust toolchain (edition 2024) and Git. ripgrep (`rg`) is optional and
only needed for content search.

```sh
cargo build --release --locked
./target/release/vyber            # or: vyber <folder>
```

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

| Action | Windows | macOS |
| --- | --- | --- |
| New tab | Ctrl+T / Ctrl+Shift+T | Cmd+T / Cmd+N |
| Split right (terminal focused) | Ctrl+D | Cmd+D |
| Split down | Ctrl+Shift+D / Ctrl+Shift+E | Cmd+Shift+D |
| Close terminal | Ctrl+Shift+W | Cmd+W |
| Next terminal | Ctrl+Shift+] | Cmd+] |
| Switch tab | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Maximize terminal | Ctrl+Shift+Enter | Cmd+Enter |
| File panel | Ctrl+Shift+B | Cmd+B |
| Find file | Ctrl+Shift+P | Cmd+P |
| Search terminal history | Ctrl+Shift+F | Cmd+F |
| Open folder | Ctrl+Shift+O | Cmd+O |
| Manual checkpoint | Ctrl+Shift+K | Cmd+Shift+K |
| Save in editor | Ctrl+S | Cmd+S |
| Copy / paste in terminal | Ctrl+Shift+C / V | Cmd+C / V |
| Settings file | Ctrl+Shift+, | Cmd+, |
| Shortcut help | Ctrl+Shift+H | Cmd+Shift+H |
| Font size up / down / reset | Ctrl++ / Ctrl+- / Ctrl+0 | Cmd++ / Cmd+- / Cmd+0 |

In the file tree: ↑/↓ to move, →/← to expand or collapse, Enter to open, Space for quick look.
In the filter box, Enter opens the best match and Shift+Enter searches file contents.

## Configuration and data

Vyber stores its data in `%LOCALAPPDATA%\Vyber` on Windows and in the user application data
directory on macOS.

| File | Contents |
| --- | --- |
| `config.toml` | `shell`, `font_family`, `font_size`, `scrollback`, `reduced_motion`, `notifications`, `restore_workspace`, `task_history_days`, `task_history_limit` |
| `workspace.json` | Tabs, split layout, terminal folders, open files and unsaved drafts |
| `checkpoints/`, `tasks/`, `recovery/` | Local review snapshots and recovery copies |
| `vyber.log` | Application errors only — terminal output is never logged |

## Known limitations

- The Claude Code / Codex session log formats are not a stable public API; if they change, the Tasks
  view may stay empty until updated. Manual checkpoints always work.
- Text preview is limited to 8 MB and file loading to 32 MB; this is not a streaming editor for huge files.
- The checkpoint object store is not compacted automatically yet, so keep an eye on its size.
- Panel widths are not restored between launches.
- No PDF or web preview, no LSP, and no hunk-level revert yet.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option. Unless you explicitly state otherwise, any contribution you submit for inclusion in
this project shall be dual licensed as above, without any additional terms or conditions.

Bundled file and UI icons come from Seti UI (MIT), Markdown Mark (CC0) and Lucide (ISC/MIT); see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for these and all Rust dependency licenses.
