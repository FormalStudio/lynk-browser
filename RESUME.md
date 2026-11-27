# Handoff (written 2026-11-27, 22:40)

All work was stopped on the owner's order. Every lane cancelled its jobs and committed what it had
on its own branch. To resume, tell Claude: "read RESUME.md and continue".

Goal: Lynk, an Arc-style Chromium browser, as a Rust app on NativeDesktop (`nd`). Usable and
stable on macOS and Linux first, Windows only as far as a guest on g815 can test it, then the
features, including [docs/inspiration.md](docs/inspiration.md). The rules for agents are
[AGENTS.md](AGENTS.md), the design is [docs/architecture.md](docs/architecture.md), the work list
is [docs/nd-parity.md](docs/nd-parity.md).

## What is pushed

- Lynk `origin/main`: `09f9685`. The owner's install on g815 is older, `2512edf`.
- nd `origin/main`: `05eb60e6` (download crash fix, toast pref and copy events, the first sidebar
  transition commits, the press fixes).
- Nothing else is pushed. A `wip:` commit is never pushed and never sent to the gate.

## The owner's open reports, in priority order

| # | Report | State |
| --- | --- | --- |
| 1 | Closing a tab that is not the active one does nothing | Cause found on macOS, fix not written. The X missed 2 of 50 clicks. The row's tracking area is `ActiveInKeyWindow`; a Chrome page's anchor window takes key on every tab switch and load, AppKit sends `mouseExited`, nothing sends `mouseEntered` back, the X goes disabled and the press falls through to the row. Fix in nd: tracking area active in the active app. Pins lane |
| 2 | Shortcuts randomly do nothing, Super+Q on g815 "randomly breaking" | Same key flip: nd redirects a key by the event's window, not the key window, so a key during the flip goes to Chromium. Fix in nd, plus a Lynk case with a page that cancels every keydown. Pins lane. Super+Q itself is formalwm (FormalShell `wm/formalwm/src/binds.rs:226`); test commit `ab143b51` on branch `xwayland-focus`, not pushed, not installed. The read-only watcher on g815 died with the reboot |
| 3 | Controls in the sidebar top strip and a sidebar toggle | Written on `top-row-d` (toggle, restart, Shift+Cmd+B, edge reveal, "Show Sidebar" in the command bar). Blocked on macOS by the modifier-key crash, fixed by nd `fa926512`, which is first in nd stack 3. Parity boxes 44 and 76 |
| 4 | Sidebar on nd's component with the moving indicator, macOS padding | Switch to nd `source_tree` at Lynk `02dc429` (branch `t3/lynk-pins-and-profiles`). Held out of `main` until report 1 is fixed. Captures beside trove still owed for a verdict |
| 5 | Sidebar toggle: page lags, white leaks; three modes docked, hidden, floating | Linux fixed and accepted (12 of 12 frames clean). macOS: the lag is Chromium placing the page by the BrowserView inset some GPU frames late. Decision: A now (cover with the page's real background, never white, at most 300 ms, drive asserts no white pixel), then B in the same lane (anchor the widget at the page so moves are same-frame; list every consumer of the window-corner assumption first). Lynk side (`SidebarMode`) on `sidebar-transition` |
| 6 | DevTools opening in a window | Every path docks on macOS and Linux. nd `ed895aa3` (branch `devtools-docked`) is in stack 3, Lynk `35d4470` needs the repin after it, `7f4b0a3` is with integration. Service worker DevTools keep Chromium's window, on purpose |
| 7 | Cut off toast pill, remake all Chromium toasts | nd half pushed. Lynk pills on `chrome-toast` (wip). The pill draws correctly on macOS; nd's AppKit probe reported every toast hidden at (0, 0). Fix on nd branch `toast-probe` (wip `62848795`), its gate not run. Captures light and dark on both platforms owed |
| 8 | e1504g low-end benchmarks | Not written. `docs/linux-bench.md` is the Linux lane's |

## Integration

- Head D `306122f`: held. Its macOS runs were at load 219 to 987.
- Head E `integrate-e` `3ffc7df` (D, nd `d2012a11` repin, `e14eb61`): macOS run 1 passed 91 of 93 at
  load 105. Failures: `real_blocking_reaches_a_page_under_a_strict_policy` and
  `real_pinned_tile_menu`.
  Linux on e1504g: 110 of 111, 110 of 111, then 88 of 111. Run 3 began with
  `real_menu_over_a_live_page` and `real_thirty_quits_in_a_row` ("the app is still running with 0
  windows"), and the rest cascaded from Lynk processes left running. The kill-and-check teardown
  of `3859e4d` covers only macOS; it needs a Linux half (shell lane), and the quit case its own
  look. Logs `e-linux-run{1,2,3}.log` are in the integration thread's scratchpad.
- Head F `integrate-f` (`lynk-wt/integrate`): E plus `chore(deps): nd 05eb60e6`, the pins pair
  (held-tile case and its settle wait, `c146a08` on `pins-held-tile-e`), `093caa0`, `ced0077`,
  `7f4b0a3`. No run yet.
- Rule for a push: no test fails in two runs on one platform, across three macOS and three Linux
  runs. A macOS run starts only below load 300. A run with the 1Password prompt in front is void.
  The orchestrator pushes with `git push origin <sha>:refs/heads/main`, then orders the g815
  reinstall.

## nd gate

- Stack 2b, nd main checkout on branch `stack2b` at `3610b66c`: extensions `129748c6..1a36f6bb`,
  `3acea6b4`, cef-argv `ce813d91` and `f78e650d`, gate-tmp `2e4d4000`. macOS passed: window group
  49 of 53 (the 4 known macOS 27 goldens), real cases 7 of 7, Lynk quit cases 4 of 4. Linux did
  not run (cancelled right after the sync). Rerun `target/gate-runs/linux-stack2.sh`, then push
  `3610b66c` by sha. The gate's own note is `target/gate-runs/RESUME-stack2b.txt` in nd. The local
  nd `main` branch still carries the dropped `1a6a5192` picks; do not push it.
- Stack 3, in order: `fa926512` (macOS modifier-key crash), `fa70a0eb` (overlay semantics),
  `c9f069cb` and `2d19f10d` (extensions), `ed895aa3` (docked DevTools), `60bbeba7` (gestures fmt),
  chrome-cursor `98485800` once cleared, then `style(nd-cef): cargo fmt` for `gpui/view.rs`,
  `mac/chrome.rs` and `windows/chrome.rs` as the last commit. From stack 4 on `cargo fmt --check`
  blocks again.
- `lynk-wt/nd-gate` is the gate's clean Lynk worktree at `09f9685`, for Lynk's quit cases against
  a local nd.

## Lanes at the stop

Verified means its own gate passed. A `wip:` head is unverified, often carries trace lines, and
is never pushed or sent on.

| Lane | Worktree and branch | Verified | Wip head and next step |
| --- | --- | --- | --- |
| shell | `lynk-wt/top-row` `top-row-d`; `lynk-wt/chrome-toast` `chrome-toast`; `nativedesktop-wt/toast-probe` `toast-probe`; main checkout `close-others`; `lynk-wt/page-card`; `lynk-wt/settings`; `nativedesktop-wt/source-dim2` | nd `fa926512` (with the gate) | `a875c4e`, `3518d09`, `62848795`, `0f398d6`, `099fbaf`, `d6d2e92`, `117281d9`. Next: run toast-probe's gate (clippy, `nav_components`, fmt) |
| pins and profiles | nd `nativedesktop-wt/source-rows` `source-rows-menu`; Lynk `~/.t3/worktrees/lynk/t3-addcdfb1-*` `t3/lynk-pins-wip` on `02dc429` | nd `2dfafac8` (context menu items in the tree), sent nowhere yet | nd `0f2ed1e2`, Lynk `ed2f004`. Next: fix the hover case's pointer mapping (it hovers row 0), then `ActiveInActiveApp` in `source_list.rs` and a key-window check in `redirect_anchor_keys`. The Linux tab table fails at case 6: the GPUI source tree takes no keyboard focus |
| tools | Lynk `~/.t3/worktrees/lynk/t3-45ef14f2-*` `t3/tools-group-b`; nd `nativedesktop-wt/tools` `devtools-docked`; nd branch `overlay-semantics` | Lynk `7f4b0a3`, `35d4470`; nd `ed895aa3`, `fa70a0eb` | `lynk-wt/csp-flake` `f4545be` (traces only). Next: under load, confirm the hidden page renderer's priority, then fix at the cause |
| extensions | Lynk `~/.t3/worktrees/lynk/t3-e65aa9d4-*` `t3/lynk-extensions-lane`; nd `nativedesktop-wt/extensions` `lynk-extensions` | Lynk `56b98b1`, `093caa0`; nd `2d19f10d`, `c9f069cb` | Lynk `019d785`, nd `42ad7d99` (per-profile extensions). The macOS case is red: Chromium's DevTools pipe answers nothing before a browser exists. Next: run `real_extension_popover_pins_and_popups` and read the `XPROBE` lines |
| command bar | `lynk-wt/tab-loads` `tab-loads`; `lynk-wt/downloads` `downloads`; `lynk-wt/opens-hang` | `e14eb61` | `843d0c6`, `50c9941`, `13f019b`. Next: why `/broken.bin` raises no failed pill; the launch freeze minidump |
| Linux | Lynk `~/.t3/worktrees/lynk/t3-ee83db20` `t3/linux-install`; nd `cef-argv`, `gate-tmp`, `chrome-cursor` | Lynk `ced0077`; nd `f78e650d`, `2e4d4000` | Lynk `cb65249`. Next: run `real_devtools_leave_the_cursor_on_the_page` on current nd and on `98485800`; fail then pass sends it to the gate |
| sidebar transition | nd `nativedesktop-wt/sidebar-transition`; Lynk `lynk-wt/sidebar-transition` | Lynk `c220a2d` (repin, `cargo check` only) | nd `79ac7c74`, Lynk `ee3cdeb`. Option A passed every dark case on macOS; light is untested. Next: rerun `real_sidebar_slide_frames` on macOS, then Hyprland, then strip traces. Option B not started |

`Cargo.lock` is left modified by local-nd builds in several worktrees. It is never committed.

Possibly still running: e1504g went unreachable at the stop, so remote builds of the gate, the
tools lane and the sidebar lane may still hold its build lock, and `~/build/nd-build/tmp-watch.sh`
ends by itself six hours after 20:55. Check e1504g before the first Linux job.

## Known failures with a cause or a lead

- `real_blocking_reaches_a_page_under_a_strict_policy`: fails 8 of 8 at load 130 to 160, 0 of 15
  at load 10. The hidden scriptlet page requests `js/resources/scriptlets.js` and goes silent.
  Lead: Chromium gives the zero-size page's renderer background priority. Tools lane.
- `real_pinned_tile_menu`: two causes, the menu items missing from the automation tree (nd change
  in progress) and the lost Cmd+1 of report 2. Pins lane.
- Linux launch freeze of 29 s, once in 20 launches; last stack is Chromium's
  `DarkModeManagerLinux::OnReadColorScheme`. Lead: a D-Bus timeout. Command bar lane.
- Downloads (`downloads` branch): two of three macOS cases pass. A cut-off download shows no
  "Couldn't download" pill. Command bar lane.
- The Linux cursor case is known on `main`; nd `98485800` fixes it.

## Hosts and locks

- macbook builds and runs macOS cases. `CARGO_BUILD_JOBS=3`, one build per lane, no build under
  the screen lock (`lockf /tmp/lynk-screen.lock`, build with `--no-run` first).
- g815 was in Windows (the owner gaming). Nobody reboots it. Its install only ever holds a pushed
  `main` commit, and the owner's running Lynk, compositor and session are never touched.
- e1504g is the Linux host until g815 is back: `ND_LINUX_HOST=e1504g`, build root
  `~/build/nd-build`, 2 jobs, `TMPDIR` on disk because `/tmp` there is a 3.7G tmpfs with a quota.
  A timeout-only failure there is marked "e1504g only" and rerun on g815.
- Build drive `/Volumes/Kyan Drive/dev-cache.noindex`: floor 15 GiB, 35 GiB free at the stop.
- The 1Password unlock prompt (process `coreautha`) is cancelled with Escape only. Nobody types
  the owner's password.

## How this is run

- One orchestrator, one agent per lane, each lane in its own worktree (`lynk-wt/<name>`,
  `nativedesktop-wt/<name>`, `~/.t3/worktrees/lynk/*`). `git worktree list` in both repos shows
  them. The main Lynk checkout is on `close-others` and belongs to the shell lane.
- nd shas go to the nd gate, Lynk shas to integration. Lanes never push.
- The 45-minute lane check (`scheduled-task:2f8d4d51-653c-4206-853a-0c1ee53b06ff`) is paused, not
  deleted. Enable it again on resume.
- Each lane's last message in its T3 thread lists its worktrees, head shas, what is verified and
  what is `wip:`.

## First steps on resume

1. Enable the lane check and resend each lane its order from the table above.
2. nd gate: rerun stack 2b's Linux half, push, then stack 3.
3. Pins lane: the two nd fixes for reports 1 and 2, each with a case that fails on the unfixed tree.
4. Integration: head F's six runs, then push and reinstall on g815 once it is on Linux.

## Owner decisions pending

- Whether Super+Q should follow the pointer across monitors, and what a miss looked like (time,
  console open, monitors).
- What "undocked" means for the sidebar beside docked and floating.
- The Windows signing identity (`[windows] signing` in `nativedesktop.toml`).
