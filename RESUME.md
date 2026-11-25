# Handoff (written 2026-11-25)

Development is running under one orchestrator with one agent per lane. To resume after a stop,
tell Claude: "read RESUME.md and continue".

Goal: Lynk, an Arc-style Chromium browser, as a Rust app on NativeDesktop (`nd`). Usable and
stable on macOS and Linux first, Windows only as far as the KVM guest on g815 can test it, then
the features, including [docs/inspiration.md](docs/inspiration.md). The plan is
[docs/delivery-plan.md](docs/delivery-plan.md) (five waves), the work list is
[docs/nd-parity.md](docs/nd-parity.md), the design is [docs/architecture.md](docs/architecture.md),
the rules for agents are [AGENTS.md](AGENTS.md). `src/` is the old TypeScript app and is only the
spec. `PLAN.md` at the root is from the WebKit era and describes nothing current.

## Where it stands

| Lane | State |
| --- | --- |
| `core` (`crates/core`) | Done and on `main`, tabs live in the core sidebar tree |
| `shell` (`crates/lynk`) | On `main`. Review findings S1 to S21 closed (four Codex rounds, one Claude round, budget spent). Session restore landed. Next: settings as a tab, then private and popup windows, permissions, external protocols |
| command bar | Merged (`4f0cf58`). Open: one run in 12 loses the first typed letters; favicons in rows, the visual pass, history, bookmarks and downloads panels, `chrome://` routes |
| Linux bring-up | Branch `t3/linux-browser-bringup`, not merged. The bar works on Linux and the build is installed on g815. Open: SIGTERM quit hang (about 1 in 20), a third client counted in the tiled case, sidebar pairs against macOS |
| Windows guest | Branch `t3/windows-vm-toolchain`, 42 of 42 three runs in the guest, ready to merge after the Linux branch |
| `package` | On `main` (`098c86a`): notarized mac app and dmg, Linux AppImage. Needs nd `0db1119b` and `4442c4cd` pushed and repinned, or the packaged mac app dies at launch |
| `blocking-tools` | Started 2026-11-25 in its own worktree: docked DevTools first, then adblock |
| `extensions` | Not started |

macOS drive on `a43068a`: 50 of 50, 49 of 50, 50 of 50. The one failure was a click sent while
macOS animated the window into fullscreen; the fix is nd `6f68fa13` plus lynk branch
`shell-fullscreen` (`52d14c8`), waiting on the nd push.

## Before any push of lynk

`main` is local only. Its history contains `28e4862 wip: shell review round 2 fixes, unverified`.
Never push a `wip:` commit: reword or squash it first, at a moment when no lane branch is in
flight, because every lane branch forks from that history.

## Gates

- core: `cargo test -p lynk-core` and `cargo clippy -p lynk-core --all-targets -- -D warnings`.
- shell drive on macOS (real windows, real pointer), three full runs in a row:
  `lockf /tmp/lynk-screen.lock env -u SDKROOT -u DEVELOPER_DIR CEF_PATH="/Volumes/Kyan Drive/dev-cache.noindex/cef" ND_REAL=1 cargo test -p lynk --test shell_drive -- --test-threads=1`.
  Every run that uses the macbook's screen goes through that lock; several agents share it.
- Linux: headless Hyprland on g815 through `scripts/linux-gate` (on the Linux branch until it
  merges): `drive 3` for three runs, `install` to put the build in the owner's session.
- Windows: the KVM guest `lynk-win` on g815, `scripts/windows-vm create|up|down` (on the Windows
  branch). Software rendering only, so corners are square there.
- Against unpushed nd: `cargo <subcommand> --config scripts/local-nd.toml ...`, with the flag
  after the subcommand. It rewrites `Cargo.lock`; never commit that.
- nd: fixed in nd, gated with the bar "no new failures against the last pushed head, fmt and
  clippy clean", pushed, then repinned here as `chore(deps): nd <sha>`. Failures that were there
  before and do not block: `nd-cli docs check`, four `layout-settings-*` Linux goldens,
  `nav.rs:591`, mac `real_resize_drag`, the Linux middle-button test.

## How this is run

- Lanes that share `crates/lynk` work in worktrees and land by rebase and merge; the shell agent
  owns `main`. Agents never push. nd experiments live in `../nativedesktop-wt/<lane>` and land by
  cherry-pick; the shared nd checkout holds only committed, compiling work.
- Every brief that touches UI names the visual spec (`docs/sidebar.md`, the old app's styles),
  the states to capture and the design skills to load. The agent judges its own captures first.
- Reviews go to Sol (Codex); when it is out of credits, to Claude Pulse on Opus.
- At each milestone the build is reinstalled on g815 for the owner to try by hand. Never kill a
  Lynk they have running.
- Worktrees to remove once their branches are merged: `lynk-wt/omnibox`, `lynk-wt/core`, the two
  `.t3/worktrees/lynk` checkouts, `nativedesktop-wt/*`.

## Owner decisions pending

- The Windows signing identity (`[windows] signing` in `nativedesktop.toml`).
- Deleting nd branches `windows-keys`, `windows-scheme`, `windows-chrome-pad`: their content is
  on `main` by cherry-pick, so git needs `branch -D`, which waits for the owner's word.
- Publishing a release: the fspec store package for Linux hosts needs a published AppImage URL.
