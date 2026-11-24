# Agent notes

Lynk is an Arc-style Chromium browser in Rust on NativeDesktop (`nd`, source at
`~/Developer/ecosystem/nativedesktop`, read only from here).

## Read first

1. [docs/architecture.md](docs/architecture.md): layers, lifecycle, registries, state, gates.
2. [RESUME.md](RESUME.md): where the work stopped. [docs/delivery-plan.md](docs/delivery-plan.md):
   the waves and lanes. [docs/nd-parity.md](docs/nd-parity.md): the boxes that define done.
3. The spec for a feature: `docs/sidebar.md`, `docs/omnibox.md`, `docs/native-pages.md`, and the
   old TypeScript app in `src/`. `src/` and `PLAN.md` are history; never edit or build them.

## Build, run, gate

On the macbook, outside a Nix shell's SDK. Prefix every cargo command that builds `lynk` with
`env -u SDKROOT -u DEVELOPER_DIR CEF_PATH="/Volumes/Kyan Drive/dev-cache.noindex/cef"`
(written `<env>` below).

- Build: `<env> cargo build -p lynk`
- Run: `<env> cargo run -p lynk` (`LYNK_URL=<url>` for the first tab)
- Core: `env -u SDKROOT -u DEVELOPER_DIR CARGO_TARGET_DIR=target/core cargo test -p lynk-core`,
  then `cargo clippy -p lynk-core --all-targets -- -D warnings`
- Shell unit: `<env> cargo test -p lynk --lib`
- Shell drive (real windows, real pointer, one at a time):
  `<env> ND_REAL=1 cargo test -p lynk --test shell_drive -- --test-threads=1`
- Against an unpushed nd, without editing manifests, add `--config scripts/local-nd.toml` after the
  subcommand (`cargo clippy --config ...`; before it, clippy drops the flag and checks the pin).
  It rewrites `Cargo.lock` in the working tree; never commit that.
- Linux (headless Hyprland on g815): `scripts/linux-gate sync`, then `scripts/linux-gate drive 3`.
  Until that script lands on `main`, use nd's `scripts/linux-gate` from the nd checkout.
- Windows (Windows 11 guest on g815): `scripts/windows-vm up`, `scripts/windows-vm sync`,
  `scripts/windows-vm run '<powershell>'`, once the Windows lane lands the script. Compile check from the macbook:
  `cargo xwin clippy --target x86_64-pc-windows-msvc -p lynk` with
  `/opt/homebrew/opt/llvm@22/bin` on `PATH`.

## Ownership and commits

- One agent owns a set of paths. Touch nothing outside it; commit only those paths
  (`git add <paths>`, never `git add -A`). Never push: the orchestrator reruns the gates and pushes.
- Never `git reset`, `checkout --`, `restore`, `clean` or `stash` on a tree with someone else's
  changes. A dirty file you did not make is another agent's work in flight.
- Subjects are short, lowercase, conventional: `feat(core): ...`, `fix(shell): ...`, `docs: ...`.
  No body unless it closes an issue. Never push a `wip:` commit.
- An nd gap is fixed in nd, gated there, pushed, then `cargo update -p nd` here in a
  `chore(deps): nd <sha>` commit. No workaround in the app.
- Tick a box in `docs/nd-parity.md` in the same commit as the test that proves it, naming it.

## Code rules

Smallest complete change:
- Ask whether it needs to exist, and whether it is already here, in nd, in std or in an installed
  dependency.
- No wrapper, option, config knob, single-implementor trait, conversion layer or code "for later".
- Deletion beats addition. Finish every caller, test and fixture in the same change.

Untangled:
- One module owns one feature and its state. Modules talk through the registries in
  `docs/architecture.md` and small typed functions, never by writing each other's fields.
- No god files, no pass-through wrappers, no duplicated logic.
- Validate at the boundary only. Make illegal states unrepresentable where a type can.
- Logic that needs no window goes in `crates/core`, which never draws.

Every change:
- One small test per new non-trivial piece of logic.
- A known shortcut carries `shortcut: <the limit>, <when to upgrade>`.
- Each module opens with a `//!` saying what it owns and what it must not do.
- Comments state only what the code cannot.
