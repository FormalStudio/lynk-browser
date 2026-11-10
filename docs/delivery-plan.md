# Delivery plan: Lynk on nd, every platform

Written 2026-11-10. Where the port stands, what is left, and the waves that close it.
[nd-port-plan.md](nd-port-plan.md) has the architecture and the module mapping,
[nd-parity.md](nd-parity.md) the boxes that define "done".

## Where it stands

- `main` carries two apps: the 0.5.11 build (`src/`, Solid on the Zig host) and the Rust
  skeleton (`crates/lynk`, 80 lines: a sidebar and one Chromium page).
- Every nd lane the port waited on is merged in nd `main`: Chrome-style pages (on Linux under
  XWayland, `nd-cef/src/linux/chrome.rs`), extensions and the Web Store, the request filter,
  private windows, user-gesture JavaScript, tab drag, sidebar reveal, print. nd's
  `examples/browser` exercises each of them, one `BROWSER_SCENE` per feature, and is the
  reference for the calls.
- Open in nd on Linux: 1Password on Hyprland, and the Web Store install prompt
  (`nd-cef/src/linux/install_prompt.rs` is a no-op, so Chromium's own prompt can follow the
  app's dialog).
- Open in nd on Windows: everything. `nd-cef` builds for macOS and Linux only,
  `nd package windows` refuses CEF, native messaging hosts are searched in macOS and Linux
  directories only. The Zig-era branch `win-cef` in the nd repo has the windowed Chrome-style
  design (`docs/superpowers/specs/2026-10-08-cef-windows.md`: `SetWindowRgn` airspace, the
  stray-window guard); `win-osr` has the off-screen path.
- No box in `nd-parity.md` is ticked. Since the 2026-10-20 decision in nd's
  `docs/plan/PROMPT.md` the port is `main` and the checklist is the work list, not a merge gate.
- Windows runs happen on real Windows (g815 booted into Windows, `ssh windows`), at a later
  date. Until then Windows code is checked by `cargo clippy --target x86_64-pc-windows-msvc`
  only, and nothing on Windows counts as done.

## Rules for every wave

- One agent owns a set of paths. It commits path-scoped on `main` and never pushes. The
  orchestrator reruns each gate itself, then pushes.
- A box is ticked in the same commit as the drive case, unit test or capture that proves it, and
  the line names it.
- Every wave's gate runs on macOS and, from wave 2 on, in headless Hyprland on g815 through nd's
  `scripts/linux-gate`. Linux is checked as features land, not at the end.
- Each wave ends with an independent review of the wave's diff (`better-call-sol`, three rounds
  at most). Blockers and shoulds are fixed before the next wave starts.
- The TypeScript app in `src/` is the spec. Nobody edits it; it is deleted in wave 4.
- An nd gap is fixed in nd (`~/Developer/ecosystem/nativedesktop`), never worked around in the
  app. nd changes are pushed by the orchestrator after nd's own tests, then
  `cargo update -p nd` here.

## Wave 1: foundation (three agents in parallel)

| Lane | Owns | Done when |
| --- | --- | --- |
| `core` | `crates/core` | Every non-drawing module of `src/lib/` is Rust. Each module has tests: the ported `*.test.ts` where one exists (url, omnibox, session, pages, permissions, downloads, favicons, webstore, reader), new ones for the modules that had none (settings keys, defaults and fallbacks; history and bookmarks against a temp SQLite file; zoom steps; tabstate verdicts; adblock list pinning and refresh rules). `cargo test -p lynk-core` passes |
| `shell` | `crates/lynk` | Window registry, tab model with lazy pages, sidebar layout, the registries later lanes append to (commands, menu items, foot slots, padlock rows, per-tab page observers), back, forward, reload, close and reopen, load bar, find, zoom, developer tools, fullscreen, new tab page, error and crashed pages, multi-window, one instance per profile, SIGTERM, Chromium bubbles off, the identity probe. `crates/lynk/tests/shell_drive.rs` drives the real app in Chrome style on macOS with one named case per box claimed |
| `cef-windows` (nd repo) | `crates/nd-cef/src/windows`, Windows wiring, `nd package windows`, native messaging through the registry, the install prompt | On real Windows with `--features cef` and `ND_CEF_STYLE=chrome`, the `examples/browser` scenes `parity`, `chrome-pages`, `extensions`, `webstore`, `adblock`, `private`, `pip` and `print` pass |

`core` lands url, settings, session, tabstate and pages first so `shell` can depend on them.
`cef-windows` ends wave 1 written and cross-checked; its gate runs with the first real Windows
session.

## Wave 2: features (five agents, each appending to the shell's registries)

| Lane | Boxes |
| --- | --- |
| `omnibox-panels` | Command bar (all three modes, ranking, inline completion), history, bookmarks and downloads panels and popover, `chrome://` routes, "Search with <engine>" in the context menu |
| `extensions` | Web Store install with the app's dialog, puzzle popover, pinned actions, popups, extension tabs, Dark Reader; then 1Password on macOS once `package` has the signed build |
| `blocking-tools` | adblock-rust on the request filter, cosmetic filters and scriptlets, the padlock's ad switch and count, element hider, reading mode, floating video and a page's own picture in picture with "Back to tab", print, save page, view source |
| `windows-session` | Session restore, settings window, private windows, popup windows, permissions and the padlock popover, blocked pop-ups, external protocols |
| `package` | `nd package mac` signed with the Developer ID (1Password trusts the signature), AppImage, the Nix package from the flake, root-owned in the store (1Password on Linux needs it) |

## Wave 3: the Arc surface (two agents, then one)

| Lane | Boxes |
| --- | --- |
| `sidebar-polish` | Pinned tile grid, Liquid Glass tiles, tab and tile drag (reorder, pin, between windows), menus on rows and tiles, edge reveal with the traffic lights, speaker and sleep states, Linux window controls from `gtk-decoration-layout` and the page card |
| `compact` | The compact layout, the layout switch, 720 to 1440 pt with nothing overlapping |
| `linux-1password`, after the two above | 1Password on Hyprland with native messaging and the Nix build, the Linux Web Store prompt, the run on e1504g in the user's session |

## Wave 4: ship (two agents)

| Lane | Done when |
| --- | --- |
| `migrate` | A copy of a real 0.5.11 profile, captured once into a fixture, opens unchanged: settings, session v1 and v2, `history.sqlite`, `blocking.json`, the Chromium profile |
| `windows-app` | Every line of the Windows section of `nd-parity.md` is ticked on real Windows: caption strip, both layouts, extensions, a packaged installer. The signing identity is the owner's choice (nd `STATUS.md`) |

Then every box is ticked, `src/` and the Bun toolchain are deleted, the README is rewritten for
the Rust build, and the nix config switches on every host.

## Wave 5: beyond parity

Arc features the 0.5.11 app never had and the 2026-09-29 note declined: spaces with their own
pinned sets and themes, folders in the sidebar, split view, auto-archive of stale tabs, peek for
links out of pinned tabs, a small window for links opened from other apps, registering as the
default browser. Each is a lane of its own after wave 4, and needs a yes from the owner first.
