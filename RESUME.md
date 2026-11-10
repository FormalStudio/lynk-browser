# Handoff (written 2026-11-10, evening)

Development is stopped. No agent is running and nothing is running on purpose. To resume, tell
Claude: "read RESUME.md and continue".

Goal: Lynk, an Arc-style Chromium browser, as a Rust app on NativeDesktop (`nd`) for macOS, Linux
and Windows. The plan is [docs/delivery-plan.md](docs/delivery-plan.md) (five waves), the work
list is [docs/nd-parity.md](docs/nd-parity.md), the architecture is
[docs/nd-port-plan.md](docs/nd-port-plan.md). `src/` is the old TypeScript app and is only the
spec. `PLAN.md` at the root is from the WebKit era and describes nothing current.

## Where it stands: wave 1 of 5, two lanes of three done

| Lane | State |
| --- | --- |
| `core` (`crates/core`, `lynk-core`) | Done, reviewed in two rounds, pushed. 17 modules, 80 tests, clippy clean |
| `cef-windows` (nd repo) | Written, reviewed, pushed in nd. Never run on Windows. macOS and Linux gates green after its refactor |
| `shell` (`crates/lynk`) | Not done. Stopped between review rounds 2 and 3 |

No box in `docs/nd-parity.md` is ticked yet. Waves 2 to 5 have not started.

## The shell lane, exactly

- Committed and unpushed on lynk `main`: `e6200d8`, `9ced343` (the shell), `7091104`,
  `766d86d` (round 1 fixes and the row menus), `2fec632` (pins nd `0d451fc1`), then
  `28e4862 wip: shell review round 2 fixes, unverified`.
- `766d86d` on nd `0d451fc1`: clippy clean, 6 unit tests pass, the drive
  (`crates/lynk/tests/shell_drive.rs`, 17 cases) passed 17 of 17 for the agent and 15 of 17 in my
  run. `real_fullscreen_page_and_browser` ("the app has a window on screen") and
  `real_put_tab_to_sleep_and_wake` ("timed out waiting for a second tab") fail in a full run and
  pass 3 of 3 alone. The log is `target/gate/shell-drive.log`.
- The agent's lead on that flake when it was stopped: a page click is lost while a Chrome-style
  page briefly has zero size after it is lifted. Its fix is nd `10242018`, unverified.
- `28e4862` is the agent's unfinished round 2 work and nobody has run it: `TAB_EVENTS` fired on
  every path, `key_window` skipping closed windows, drive changes. It builds only against the two
  unpushed nd commits below. Never push a `wip:` commit; finish it, verify, then squash or reword.
- nd, local `main` at `~/Developer/ecosystem/nativedesktop`, two commits unpushed and not gated
  by me: `e27c2d62` (a right click on a list row no longer selects it) and `10242018`.

Open review findings for the shell (round 2, Claude Opus):
- S3 should: `TAB_EVENTS` is not called for tabs that start a window, for a window close, or
  correctly for moves. Wants a unit test with a recording listener.
- N1 should: a right click on a tab row selects it, so the browser switches tabs. The nd fix is
  `e27c2d62`; the drive must open the menu on a tab that is not shown and assert the shown tab.
- S4 nit: a launch arriving as the last window closes loses its URLs (`app.rs`, `key_window`).
- For the Linux lane: F11 cannot leave a fullscreen the window manager started.

## Next steps, in order

1. Finish the shell: get the full drive to pass three times in a row with `--test-threads=1`,
   close S3, N1 and S4.
2. Gate nd (commands below), push nd, `cargo update -p nd` here, rerun the drive on the pushed nd.
3. Review round 3 (the last in budget) on `git diff 766d86d HEAD -- crates/lynk` plus the two nd
   commits. On approval push lynk.
4. Tick the shell's boxes in `docs/nd-parity.md`, each naming its drive case.
5. Start wave 2 (five lanes, `docs/delivery-plan.md`). The seams they append to:
   `actions::COMMANDS`, `MENU_ITEMS`, `CONTEXT_KEYS`, `sidebar::FOOT`, `site_info::ROWS`,
   `page::OBSERVERS`, `TAB_EVENTS`, `shell::OVERLAYS`, `BrowserWindow::open_tab`,
   `app::open_window`.

Carried into wave 2 from the `core` review: nothing calls `ublock::scriptlet_resources` yet. The
`blocking-tools` lane has to evaluate the release's `js/resources/scriptlets.js` in a hidden page,
pass `builtinScriptlets` through it and `save_resources`, or `+js()` filters do nothing on a
fresh profile. Declined nits: the filter-list date fallback, lazy favicon ink.

## Gates

- core: `env -u SDKROOT -u DEVELOPER_DIR CARGO_TARGET_DIR=target/core cargo test -p lynk-core`,
  and `cargo clippy -p lynk-core --all-targets -- -D warnings`.
- shell drive (macOS, opens real windows, moves the real pointer):
  `env -u SDKROOT -u DEVELOPER_DIR CEF_PATH="/Volumes/Kyan Drive/dev-cache.noindex/cef" ND_REAL=1 cargo test -p lynk --test shell_drive -- --test-threads=1`.
  Captures land in `target/tmp/shell-*.png`; look at the 1200 and 720 pt ones.
- Against unpushed nd, without editing manifests: add
  `--config 'patch."https://github.com/FormalStudio/NativeDesktop".nd.path="../nativedesktop/crates/nd"'`.
- Windows cross-check from the macbook: `cargo xwin clippy --target x86_64-pc-windows-msvc -p lynk`
  with `/opt/homebrew/opt/llvm@22/bin` on `PATH` (gpui's resource build needs `llvm-rc`).
- nd on Linux, from the nd checkout, headless Hyprland on g815:
  `scripts/linux-gate sync`, then
  `ND_LINUX_GATE_TIMEOUT_MIN=150 scripts/linux-gate run 'cargo build -p browser && cargo test -p nd-os-linux --features cef --test real_webview -- --test-threads=1'`.
  Expect 18 passed. Without `cargo build -p browser` first, 17 fail at setup.
- nd on macOS for the list change:
  `ND_REAL=1 cargo test -p nd-backend-appkit --lib --test notes_gaps`. Do not run the whole crate
  without the `cef` feature: its extensions gate refuses to start.

## Windows

- There is no Windows VM any more. It was removed from g815 and the fspec spec on 2026-11-10
  (dotfiles `851e3b3c`). Windows runs happen on the Windows PC (g815 booted into Windows,
  `ssh windows`); nd's `scripts/windows/remote` drives it.
- Nothing in nd's Windows engine has run. The first run, in the nd checkout on the PC, once per
  scene (`parity`, `chrome-pages`, `extensions`, `webstore`, `adblock`, `private`, `pip`, `print`):
  `$env:ND_WEBVIEW_ENGINE='cef'; $env:ND_CEF_STYLE='chrome'; $env:BROWSER_SCENE='parity'; cargo run -p browser --features cef`
  and look for `ND_..._OK` on stderr (`ND_PRINT pdf=` for print).
- To check on that run (also in nd `docs/plan/TODO.md`, lane `cef-windows`): the extension popup
  search can take a tooltip for the popup; the install prompt presses Cancel in an RTL locale;
  whether Chromium finds 1Password's native messaging host through Chrome's registry key.
- Owner's call, still open: the Windows signing identity (`[windows] signing = "azure"` or
  `"certificate"` in `nativedesktop.toml`).

## How this was run

- One orchestrator, one Opus agent per lane, lanes owning disjoint paths in the main checkouts
  (no worktrees), path-scoped commits, agents never push. The orchestrator reruns every gate
  before pushing; agent reports were wrong about a full-run drive once already.
- Reviews: Sol (Codex) was out of credits and Claude Pulse was rate limited, so every review ran
  on the main Claude account with Opus. Three rounds at most per lane.
- nd gaps are fixed in nd, gated on macOS and Linux, pushed, then repinned here with
  `cargo update -p nd` in a `chore(deps): nd <sha>` commit.
- The macbook is where drives run; they need the screen, so one drive at a time.

## Owner decisions pending

- Wave 5 (spaces, folders, split view, auto-archive, peek, a small window for links from other
  apps, default browser) was declined on 2026-09-29 and stays out until the owner says yes.
- The Windows signing identity.
