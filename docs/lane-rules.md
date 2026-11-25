# Rules for every lane agent

Read with [AGENTS.md](../AGENTS.md). The orchestrator's brief names your lane and its order of work.

## Trees

- Work in your own lynk worktree, branched from `main`. Rebase onto `main` before each report
  (plain `git rebase main`; resolve conflicts by hand, never by discarding). Do not push.
- nd changes go in your own nd worktree, made with `git worktree add ../nativedesktop-wt/<lane>`
  from `~/Developer/ecosystem/nativedesktop`. Commit there, fmt and clippy clean, report the shas.
  Never push nd and never leave uncommitted edits in the shared nd checkout.
- Build lynk against your nd worktree with `cargo <subcommand> --config <your copy of
  scripts/local-nd.toml> ...`, the flag after the subcommand. It rewrites `Cargo.lock`; never
  commit that rewrite.

## Uncommitted work is never yours to discard (hard rule)

Modified, staged and untracked files in a tree are work in flight: the owner's, or another agent's
running beside you. Never run, on any tree, with any flag, for any reason short of the owner's
explicit instruction naming the command:

* `git reset --hard`, `git reset` on paths you did not stage yourself, `git checkout -- <path>`,
  `git checkout .`, `git restore`, `git clean`.
* `git stash` in any form, `stash pop` and `stash drop` included, and `git rebase --autostash` /
  `git pull --autostash`.
* `git worktree remove --force`, `git branch -D` on an unmerged branch.
* `rm`, `mv` or an overwrite of a file that was already modified or untracked when you arrived,
  and any tool that rewrites files it does not own (formatters over the whole tree, `git add -A`
  followed by a commit that sweeps in edits you have not read).
* Any "fix" for a dirty tree, a blocked `git switch`, a pull or rebase conflict, or a build that
  cannot see untracked files, that works by discarding changes. The fix is always to stop and
  report the state.

Before the first write to a tree that `git status --porcelain` shows is not clean: read the
status; every path you did not create belongs to someone else. "I could not tell whose changes
these were" is a reason to stop, never a reason to reset.

## Shared machines

- Every run that uses the macbook's real screen is wrapped `lockf /tmp/lynk-screen.lock <command>`.
  Several agents share that screen: run single cases while developing, a full drive only before a
  report.
- macOS drive: `env -u SDKROOT -u DEVELOPER_DIR CEF_PATH="/Volumes/Kyan Drive/dev-cache.noindex/cef" ND_REAL=1 cargo test -p lynk --test shell_drive <filter> -- --test-threads=1`.
- Linux runs happen in a headless Hyprland on g815 (`scripts/linux-gate`; until the Linux branch
  merges it lives in `/Users/kyandesutter/.t3/worktrees/lynk/t3-ee83db20`, read only for you).
  Check `meshtop hosts status` first. g815 is the owner's desktop: never stop, restart or disturb
  their apps or session, and never kill a Lynk they have running. Never compile on e1504g.
- Build output stays in the repo's git-ignored `target/` (or `~/build` on g815).

## Code

- Non-drawing logic goes in `crates/core` with tests. The shell appends to its registries
  (commands, menu items, foot slots, padlock rows, page observers, tab events) and does not edit
  other features' code.
- The smallest complete change: no speculative options, mirror the nearest sibling, delete what
  you replace. Load the `ponytail` and `codebase-slop` skills and check your diff against them.
- Document each feature in `docs/architecture.md`. Tick each box you proved in
  `docs/nd-parity.md`, naming the drive case.
- `cargo fmt --check` and clippy on touched files before each commit.

## Tests

Real inputs and real edge cases, in the drive (`crates/lynk/tests/shell_drive.rs` and its `os/`
helpers). Assert against local fixture servers, not the public internet, except where the feature
is the public service itself (the Chrome Web Store). A failing wait gets a cause, never a longer
wait.

## UI

The owner rejected earlier chrome as "looks bad". For every visible piece: load the skills
`better-ui` and `emil-design-eng` before writing it; take sizes, spacing, colours and type from
`docs/sidebar.md` and the old app's styles in `src/`; capture rest, hover, pressed, focused,
disabled where it exists, long text, a 720 pt wide window, light and dark. Judge your captures
against Arc and Safari before reporting, fix what looks default or cramped, and send before and
after pairs as file paths.

## Text

- Commits: short imperative lowercase conventional subject, no body, no attribution trailer of
  any kind. Never touch git identity (`git config user.*`, `--author`, author or committer
  environment variables).
- No em dashes anywhere. Never the phrase "smoke test". No secrets in commits or output.
- Reports carry results only: branch and shas, drive result lines per platform, capture paths,
  what is known broken.
