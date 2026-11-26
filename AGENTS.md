# AGENTS.md — Marky

Marky is a Windows Markdown editor built in Rust with [GPUI](https://gpui.rs/) (Zed's GPU UI framework), with no webview. Its reason to exist is being small and fast. `docs/gpui-migration.md` records how the port was measured and what was decided; this file is the working guide. The app code lives in `src/`, and test, benchmark and release scripts in `scripts/`.

## Decisions

`DECISION.md` records each trade-off with the options that were rejected and why, newest first. Read it before reopening a settled question (a dependency, a budget, which thread does some work, an approach already tried); change course only with new evidence, in a new entry. When a change picks one approach over another that a later reader might try again, add an entry with the rejected options. Work found but not done goes in `TODO.md`.

## Architecture invariants

- **Rust owns the disk and the UI.** Documents live in a rope in `buffer.rs`; the editor state, undo history, tabs and selection are Rust. There is no IPC boundary and no webview. Typing stays in the GPUI view.
- **Paths come from the user.** Dialogs (`rfd`), drag and drop, CLI arguments, the recent list, recovery and the single-instance request file are the only ways a path enters. Relative links go through `allowlist::link_target`, which follows only existing `.md`, `.markdown` and `.txt` files (including paths that use `..`). The preview never navigates.
- **Text model.** The buffer always holds LF text. `files.rs` detects and preserves BOM and line endings, so saving an unedited file reproduces it byte for byte. Every write, documents and JSON files alike, goes through `files::write_atomically` (a temp file unique per write, `sync_all`, atomic `fs::rename`). `killed_mid_save_leaves_old_or_new_content` guards this.
- **Opening is size-limited.** `files::read` refuses files over `MAX_OPEN_BYTES` (128 MB) before reading them, and stops the read at the limit in case the file grows in between. Every way of opening goes through it.
- **Recovery and recent files reuse the app's formats.** `store.rs` writes `recovery.json`, `recent.json` and `window.json`; recovery snapshots are written every 30 s only when a buffer version changed, are held until the user explicitly restores or discards them, and go through one writer thread so they reach the disk in order (close waits for it). At startup the file path is read before the window opens, so the first frame contains the document.
- **Diagnostics stay on the machine.** `diagnostics.rs` writes `marky.log` in `%LOCALAPPDATA%\dev.marky.editor`: panics (release builds abort and have no console), failed queued writes, and background write failures. Nothing is sent anywhere, and the log is capped. Route a swallowed error that could cost data through it rather than `let _ =`.
- **External changes are recognised by mtime.** `watcher.rs` watches each open document's folder (saves replace files by rename). Clean documents reload; dirty ones prompt Reload / Keep mine. A save updates the document's mtime, so our own saves never prompt.
- **Single instance.** `instance.rs` holds an exclusive handle on `instance.lock` (`share_mode(0)`); a second launch writes its path to `open-request.txt` and exits. Opening the lock briefly retries, because a just-exited process can hold it for a few milliseconds.
- **Tabs have independent state.** Each document has its own buffer, undo history, scroll positions, watcher and — in `Prototype::views` — its own cached preview and outline views. The active tab is the one the view reads; switch only through the tab helpers.
- **One command table.** Menu items, toolbar buttons and keyboard shortcuts all run a `Command` in `editor/chrome.rs`. Shortcuts are registered in `src/main.rs` (`key_bindings`); menu mnemonics are actions, not key listeners, so GPUI consumes the key-down instead of typing it. A new user-facing action gets a menu entry, a shortcut where the app has one (Quit, Auto Save and Larger/Smaller Text are menu- and settings-only), and an end-to-end check where practical. The bar carries File, Edit, View and Help; Help holds About, the update check and the project link, as in other Windows apps.
- **Only the update check talks to the network, and only when asked.** `update.rs` sends one `GET` to GitHub's `releases/latest` through WinHTTP (`windows-sys`'s `Win32_Networking_WinHttp` feature), so the program carries no HTTP client. It runs on the background executor, never sends anything about the document or the machine, and is started by the user from Help > About Marky. Nothing else in the app reaches the network.
- **Settings have one store.** `settings.rs` owns the schema and defaults, reads saved JSON field by field, and writes `settings.json` atomically to `%APPDATA%\dev.marky.editor`. Unknown keys are preserved. A first run may read the old prototype's `dev.marky.gpui-prototype` file once, but never writes it. A new setting needs a field, a default, a reader line, a control in the settings panel and a save.
- **Colours are roles.** `theme.rs` owns every colour: a view names a `Color` role (`Base`, `Card`, `Stroke`, `Accent`…) and `theme::color(dark, role)` returns it, so no view holds a hex. The values are WinUI 3's, and the accent is the user's Windows accent (`accent.rs`, read from the registry once a second, falling back to `#0078D4`), with a separate `AccentText` role for accent-as-text where the fill would fail 4.5:1. `contrast.rs` reads the real palette and checks every text pair at 4.5:1 in both themes and the focus ring at 3:1; `high_contrast.rs` maps each role to a Windows system colour by an explicit table, never by guessing from brightness. Any new colour is a new role with a value in both palettes. Mermaid and math bitmaps are theme-specific and are dropped when the theme changes.
- **Markdown, not a platform's flavour.** The preview renders CommonMark, the GFM basics every tool supports (tables, task lists, strikethrough, autolinks), and syntax several unrelated tools share: footnotes, definition lists, abbreviations, `~sub~`/`^sup^`, `==mark==`, math and Mermaid. GitHub alerts, emoji shortcodes, GitHub heading ids, VuePress containers and markdown-it's `++inserted++` stay out. The test: several unrelated tools render it, and it still reads well in tools that do not.
- **`src/` is grouped by subsystem.** `editor.rs` is the window view and its state, with `editor/` holding one module per concern: `document.rs` (a document's buffer, selection and parse results), `tabs.rs` (the open documents and the active one), `editing.rs` (buffer changes, caret, selection, parse scheduling), `disk.rs` (saving, opening, the watcher), `closing.rs` (the discard prompt and window close), `input.rs` (Windows text input and IME), `links.rs` (following a preview link), `window.rs` (saved geometry, title, frame timing), `find.rs`, `recovery.rs`, `drag.rs`, `draw.rs` (line rendering, the caret follow and the source pane's painting helpers), `source_pane.rs` (the source view: shell, focus, IME and a11y), `preview_pane.rs` (a document's cached preview view), `outline_pane.rs` (a document's cached outline view), `scrollbar.rs` (the panes' overlay indicators), `a11y.rs`, `lists.rs`, and `chrome/` for the window frame: `menu.rs`, `settings.rs`, `recent.rs`, `about.rs` (the About card and its update button), `title_bar.rs` (the caption row, tabs and window buttons) and `mod.rs` (the `Command` table, toolbar, tabs, outline, status bar); `preview.rs`, `preview_view.rs`, `extras.rs`, `math.rs`, `diagrams.rs` and `syntax.rs` hold the preview and the editor's highlighting (both from one parse); `bitmaps.rs` draws an SVG into the bitmap both caches keep and drops the ones no document shows any more; `text_field.rs` is the find bar's field, `icons.rs` the toolbar's SVGs; `settings`, `theme`, `accent`, `contrast`, `high_contrast`, `diagnostics`, `instance`, `perf`, `store`, `watcher`, `files`, `allowlist` and `update` (the GitHub release check) have no UI. `main.rs` wires them together. `editor/disk.rs` is not `files.rs`: the name keeps it from shadowing the crate-level `files` module it calls, and `chrome/settings.rs` refers to `crate::settings` for the same reason.
- **Preview updates by block.** The preview is parsed on a background thread after a pause; each top-level block carries its source line, and only changed blocks are replaced in the preview pane's list. Parsing uses pulldown-cmark plus `extras.rs` for what it lacks. Keep the line map: synced scrolling, outline jumps and `#fn:` links depend on it.
- **Math renders in Rust.** `math.rs` uses RaTeX and refuses anything `validate_layout` rejects: RaTeX's output-work budget, 65,536 estimated drawing items, plus a non-finite dashed-separator check. A pathological formula is an error, never an unbounded layout; broken TeX shows its source and the error. An inline `$` must hug its content, so prices stay text. Formulas are cached by size, text colour and theme; unused bitmaps are dropped after each parse and on font-size/theme changes.
- **Mermaid renders in process.** `diagrams.rs` uses merman with `securityLevel: 'strict'` and `htmlLabels: false`, on the background thread, only for diagrams the virtualised preview shows. Renderers are cached per theme per worker thread; merman's 5 s deadline reports a diagram it cannot finish. The SVG is rasterised here so unused bitmaps are dropped with the rest of the preview's images.
- **The editor, preview and outline are virtualised GPUI lists.** Cost follows the screen, not the file. The preview and outline are child views cached with `Entity::cached`, so a keystroke redraws only the editor; anything that changes them must notify them (a parse, theme/font, folder, focus, scroll, accessibility activation or a settings change that reveals them). Each pane draws a thin scrollbar when its content overflows (`editor/scrollbar.rs`). Source lines are `StyledText`; clicks, the caret and the IME candidate window use GPUI's text layout. The preview pause scales with parse time, 30–150 ms. Measure with `scripts/bench-large.ps1` before changing either bound.
- **Pane edges are draggable.** The outline width and the editor/preview split are moved by six-pixel handles in the window (`PaneResize`), with a one-pixel divider so the boundary is visible. They are per-session state, not settings; keep the minimums (outline 140–400 px, split 15–85%) when adding a pane.
- **The source editor is a view, not the window.** `source_pane.rs` owns the editor shell, its focus handle (shared with the window), the `Editor` key context, Windows text input/IME and the editor's accessibility node. The buffer, selection, undo, find matches, painted layouts and `render_line` stay on the window, so editing rules live in one place; the view asks the window to build each visible line. The source view is deliberately uncached — a keystroke must redraw it — and the editing-key handlers wrap only this view, never the window, so a Backspace in the find field cannot reach the document.
- **The program carries its icon.** `build.rs` embeds `assets/marky.ico` (through `assets/marky.rc`); Explorer, the taskbar, shortcuts and the `.md` association read it, and the installer shows the same file. The SVGs are the artwork, `app-icon.svg` and `app-icon-small.svg` for 16–24 px where the arrow would merge with the M; `scripts/make-icon.ps1` regenerates the committed .ico for every size the shell asks for.
- **Window geometry is persisted.** `main.rs` restores `window.json`; `schedule_geometry_save` writes after a move or resize settles, not every frame. The window is created in code with the saved theme.
- **Accessibility is part of the UI.** Editor text runs carry painted bounds and per-character x offsets/advances so UI Automation has a real caret rectangle. The preview exposes a document with synthetic text runs for character/word/line review, and is one tab stop (Left/Right select links, Enter follows). The settings panel traps focus; menu access keys are underlined and registered actions. A new control needs a role, a name and its state.

## Performance budgets

Budgets are release-blocking. The README's **Benchmarks** section is the single record of current numbers; update that table, its date and the measurement sentence after any benchmark run.

- Program `marky.exe`: ≤ 24 MB hard, ≤ 22 MB target. It includes all Mermaid diagram types; the accepted overrun is recorded in `DECISION.md`.
- Installer: ≤ 6 MB target, ≤ 10 MB hard.
- Startup ≤ 900 ms target / 1,000 ms hard; memory ≤ 35 MB target / 60 MB hard; typing ≤ 20 ms target / 33 ms hard; preview ≤ 200 ms target / 400 ms hard.

Measure a release build (`cargo build --release --workspace`), then `./scripts/bench.ps1`; large files use `./scripts/bench-large.ps1`; the launch-flash check is `./scripts/bench-flash.ps1`. Dev-build numbers are meaningless.

Benchmark pitfalls, each of which has produced a wrong conclusion here: (1) a crash-recovery snapshot left in `%LOCALAPPDATA%\dev.marky.editor\recovery.json` is loaded at every launch, adds memory and can pop a recovery prompt. The e2e harness uses temporary folders; a manual run should delete it and any `instance.lock` first. (2) Windows trims an untouched window's working set, so RAM differs before and after use; `bench-large.ps1` reports both. (3) Background load moves typing latency by up to 2× between sessions; compare only within one session. (4) PowerShell `Process.MainWindowHandle` can return a hidden window; find the visible window by class or enumerate visible windows. (5) Don't wait a fixed time for the preview or the window; wait for the status bar or a title change (`Wait-Until` in `scripts/lib.ps1`). (6) `SendKeys` needs the window in front; `-Via Messages` is useful for stalls but its typing numbers are not comparable.

Large documents: virtualisation keeps typing and startup independent of file size, but text, parsed blocks, line index and images still grow with the document (about 13 MB per MB before syntax highlighting). The preview builds all blocks, so idle memory grows with the document; measure before and after any preview change.

## Verification

A change is done when these pass (the same commands are in the `Makefile`):

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

```bash
cargo build --release --workspace
```

For colour or theme changes, the contrast test is part of `cargo test`. For file handling, window lifecycle, shortcuts or packaging, also run the end-to-end smoke test against a release build:

```bash
./scripts/e2e.ps1
```

Every line must read PASS or SKIP with a documented reason. Native dialogs block the harness, so it covers only dialog-free paths; extend `scripts/e2e.ps1` when adding a flow it can drive. After packaging changes, run `./scripts/build-installer.ps1`, `./scripts/check-size.ps1` and `./scripts/package-portable.ps1`. CI (`.github/workflows/ci.yml`) runs clippy, tests, the notice check, the release build, the installer, size budgets, e2e and the portable ZIP on pushes to `main` and `dev` and on pull requests. Harness scripts wait for state, never a fixed time: a cold CI runner starts GPUI far slower than this laptop.

## Releasing

Every user-visible change adds a line under `## [Unreleased]` in `CHANGELOG.md` (Keep a Changelog headings: Added, Changed, Fixed, Removed, Security), written for users, not as a commit log. `release.ps1` refuses an empty Unreleased section and stamps it with the version and date; `release.yml` publishes that section as the release notes.

Versions are plain `X.Y.Z`. `Cargo.toml` is the only source: `scripts/release.ps1` syncs `Cargo.lock` and the README badge. Never hand-edit versions or create tags directly. Create a release with `./scripts/release.ps1 -Version X.Y.Z` (or `make release VERSION=X.Y.Z`), then push with `git push origin HEAD --follow-tags`. A pushed `vX.Y.Z` tag publishes a GitHub Release through `release.yml`, which first runs the whole of `ci.yml` on the tag and publishes the artifacts that run built and tested; never add a publish path that skips it. Suffixes such as `-beta.1` are rejected; NSIS and winget both prefer plain numbers.

The installer is per-user NSIS (`installer/marky.nsi`), built by `scripts/build-installer.ps1`; it carries Marky, the licence and the notices, and registers the app with **Open with**. The portable ZIP carries the same files. Releases ship **unsigned** until a signing command is configured. Signing is route-agnostic: `scripts/sign.ps1` runs `MARKY_SIGN_COMMAND` (with a `{file}` placeholder), and `scripts/build-signed.ps1` builds, signs the program before the installer, signs the installer, then verifies every Authenticode signature. Keep certificates, credentials and the command in CI secrets only. winget manifests come from `scripts/winget-manifest.ps1` (schema 1.12.0, validated with `winget validate`).

## Dependencies and licensing

Marky is MIT. After adding, removing or upgrading any Rust dependency, run `cargo run -p xtask -- notices` (`make notices`) and commit the regenerated `THIRD_PARTY_NOTICES.txt`; CI fails if it is stale. The `xtask` workspace member reads `cargo metadata` and `cargo tree` and ships nothing. The file ships in the installer and portable ZIP. Prefer MIT, Apache-2.0, BSD, ISC or Zlib dependencies. Flag anything copyleft (GPL, LGPL, AGPL) before adding it. GPUI is Apache-2.0 and pinned to one Zed commit (`Cargo.toml`); moving the pin needs a build, the tests and a startup measurement, and an entry in `DECISION.md`. Zed's own editor/markdown crates are GPL: read them, do not copy them.

## Environment gotchas

- Shells are PowerShell on Windows. Shells opened before Rust was installed can lack cargo on PATH; prepend `$env:USERPROFILE\.cargo\bin`.
- `cargo run --release` without a path opens an Untitled tab, as the Start menu shortcut does; pass `scripts/fixtures/typical.md` (or use `make run`) for the fixture. Never default to a path from the build machine: `env!` paths are baked into the shipped program.
- NSIS is needed for `build-installer.ps1`; CI installs it with Chocolatey. `makensis` from PATH or the standard NSIS install folders is used.
- There is no Node or frontend toolchain: notices come from the `xtask` workspace member (`cargo run -p xtask -- notices`), and `cargo build --release --workspace` builds it alongside Marky. Only `marky.exe` is packaged.

## Code style

Pragmatic, readable code that balances SOLID and DRY without overengineering:

- Guard clauses and early returns (`let … else { return }` in Rust) keep functions flat and reject invalid state first.
- Names reveal domain intent (`files::write_atomically`, `schedule_geometry_save`, `apply_settings`).
- Add an abstraction when a second caller needs it, and reuse the ones that exist: `files::same_path`, `allowlist::link_target`, `store`'s JSON readers, `scripts/lib.ps1` for process trees and waits.
- Comments explain *why*: a budget, a security reason, a platform quirk.
- The preview and editor must agree on what is markup; share the parser's rules rather than duplicating them.
- Formatting is by hand. The code prefers long lines and is not rustfmt-clean, so there is no formatter target and CI does not run `cargo fmt`.
