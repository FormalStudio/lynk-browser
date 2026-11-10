# Delivery plan: Lynk on nd, every platform

Written 2026-11-10. Where the port stands, what is left, and the waves that close it.
[nd-port-plan.md](nd-port-plan.md) has the architecture and the module mapping,
[nd-parity.md](nd-parity.md) the 187 boxes that define "done" for macOS and Linux.

## Where it stands

- `main` carries two apps: the shipping 0.5.11 build (`src/`, Solid on the Zig host) and the
  Rust skeleton (`crates/lynk`, 80 lines: a sidebar and one Chromium page).
- Every nd lane the port waited on is merged in nd `main`: Chrome-style pages, extensions and
  the Web Store, the request filter, private windows, user-gesture JavaScript, tab drag, sidebar
  reveal, print. nd's `examples/browser` exercises each of them and is the reference for the
  calls.
- Open in nd: the XWayland path on Hyprland that 1Password needs, and Windows. `nd-cef` builds
  for macOS and Linux only. The Zig-era branches `win-osr` and `win-lynk` in the nd repo show
  how the 0.5 host did it (off-screen pages into a swap chain, IME through a text services edit
  context).
- No box in `nd-parity.md` is ticked. Since the 2026-10-20 decision in nd's `docs/plan/PROMPT.md`
  the port is `main` and the checklist is the work list, not a merge gate.
- g815's root is at 97% (8.5 GB free on 2026-11-10). The Windows VM needs 45 GB free where its
  overlay lives, so Windows runs wait on a cleanup there.

## Rules for every wave

- One agent owns a set of paths. It commits on the branch it was given, path-scoped, and never
  pushes. The orchestrator reruns each gate itself, then merges and pushes.
- A box is ticked in the same commit as the drive, unit test or capture that proves it.
- Each wave ends with a review by Sol (`better-call-sol`, three rounds at most) on the wave's
  diff. Blockers and shoulds are fixed before the next wave starts.
- The TypeScript app in `src/` is the spec. Nobody edits it; it is deleted in wave 4.
- An nd gap is fixed in nd (`~/Developer/ecosystem/nativedesktop`), never worked around in the
  app. nd changes are pushed by the orchestrator after nd's own tests, then
  `cargo update -p nd` here.
- macOS checks run on the macbook, Linux through nd's `scripts/linux-gate` on g815 (headless
  Hyprland), Windows in the Windows VM on g815.

## Wave 1: foundation (three agents in parallel)

| Lane | Owns | Done when |
| --- | --- | --- |
| `core` | `crates/core` | Every non-drawing module of `src/lib/` is Rust with its tests ported (url, settings, session, tabstate, pages, omnibox, history, bookmarks, favicons, downloads, permissions, zoom, webstore, adblock lists); `cargo test -p lynk-core` passes |
| `shell` | `crates/lynk` | Window registry, tab model with lazy pages, sidebar layout (controls row, tab rows, foot), actions and the menu bar, back, forward, reload, close and reopen, load bar, find bar, zoom, new tab page, load error and crashed pages, multi-window, all on Chrome-style pages; an `nd-test` drive of the real app passes on macOS |
| `cef-windows` (nd repo) | `crates/nd-cef/src/windows`, its wiring | nd's `examples/browser` parity scene passes in the Windows VM |

`core` lands url, settings and session first so `shell` can depend on them early.

## Wave 2: features (four agents, one module each on the shell's seams)

| Lane | Boxes |
| --- | --- |
| `omnibox-panels` | Command bar (all three modes, ranking, inline completion), history, bookmarks and downloads panels and popover, `chrome://` routes |
| `extensions` | Web Store install with the app's dialog, puzzle popover, pinned actions, popups, extension tabs, Dark Reader, 1Password on macOS with the Developer ID build |
| `blocking-tools` | adblock-rust on the request filter, cosmetic filters and scriptlets, per-site switch, element hider, reading mode, floating video, print, save page, view source |
| `windows-session` | Session restore, settings window, private windows, popup windows, permissions and the padlock popover, blocked pop-ups, external protocols |

## Wave 3: the Arc surface (three agents)

| Lane | Boxes |
| --- | --- |
| `sidebar-polish` | Pinned tile grid, Liquid Glass tiles, tab and tile drag (reorder, pin, between windows), menus on rows and tiles, edge reveal with the traffic lights, speaker and sleep states |
| `compact` | The compact layout, the layout switch, 720 to 1440 pt with nothing overlapping |
| `linux` | The whole checklist in headless Hyprland on g815: XWayland pages, window controls from `gtk-decoration-layout`, the card, 1Password with native messaging |

## Wave 4: ship (three agents)

| Lane | Done when |
| --- | --- |
| `package` | `nd package mac` signed with the Developer ID, AppImage, the Nix package replacing the `requireFile` derivation, rebuilt on macbook and g815 |
| `migrate` | A 0.5.11 profile opens unchanged: settings, session v1 and v2, `history.sqlite`, `blocking.json`, the Chromium profile |
| `windows-app` | Lynk runs in the Windows VM on `cef-windows`: caption strip, layouts, extensions, a packaged installer |

Then every box is ticked, `src/` and the Bun toolchain are deleted, the README is rewritten for
the Rust build, and the nix config switches on every host.

## Wave 5: beyond parity

Arc features the 0.5.11 app never had and the 2026-09-29 note declined: spaces with their own
pinned sets and themes, folders in the sidebar, split view, auto-archive of stale tabs, peek for
links out of pinned tabs, a small window for links opened from other apps, registering as the
default browser. Each is a lane of its own after wave 4, and needs a yes from the owner first.
