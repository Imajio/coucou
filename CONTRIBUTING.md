# Contributing to Coucou

Thanks for wanting to help Mochi grow up! 🫶

The rules below are short and mandatory. They exist so the repository's history
can be read a year from now, not excavated.

## Getting started

```bash
brew install xcodegen
cd NotchBuddy && xcodegen && open NotchBuddy.xcodeproj
```

Never edit `NotchBuddy.xcodeproj` by hand: change `project.yml` and run `xcodegen`.

Check resting island dimensions on screens with and without a notch:

```bash
bash scripts/test-screen-geometry.sh
```

The Windows and Linux app (Tauri, in `windows/`) is described in
[`windows/README.md`](windows/README.md).

## Good first contributions

- A new service integration (a poller + an entry in `PillCatalog.swift` in the `.service` category + a detail card). Look at `StripePoller.swift` for a compact example.
- A new agent: any agent already gets its own automatic pill by sending `coucou_agent` in its hook payload (see `docs/AGENTS.md`). Add an entry in `PillCatalog.swift` in the `.agent` or `.workspace` category only if you want it to be declarable in Settings → Active pills.
- A new emote or sound for Mochi.
- Bug fixes - please describe how to reproduce.

## Rules of the house

- Swift 6, SwiftUI + AppKit, **no third-party dependencies** unless there's really no other way.
- Secrets go in the Keychain, never on disk or in git.
- No telemetry, no network calls except to services the user configured.
- Never block Claude Code: if the app doesn't answer, the hook must exit right away.
- Never write `~/.claude/settings.json` without a backup and the user's confirmation.
- Keep it light: 0 % CPU when the island is hidden.

## Branches

The trunk is `main`. Nothing is committed to it directly: all work happens on
a short-lived branch and comes back by merge.

| Prefix | For |
|---|---|
| `feat/` | new functionality |
| `fix/` | a defect fix |
| `docs/` | documents only |
| `chore/` | build, dependencies, tooling |
| `refactor/` | a change of form without a change of behaviour |

A branch name is the prefix plus a short subject, hyphen-separated:
`feat/stripe-refunds`, `fix/hook-timeout-deny`.

A branch lives days, not weeks. A long-lived branch is not thoroughness - it
is a merge conflict deferred.

## Commits

Format - [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <what was done, imperative mood, under 72 characters>

<why this was done; what would break without it; what it costs>
```

Types: `feat`, `fix`, `docs`, `test`, `refactor`, `build`, `chore`.
Scope is the part of the repository the change lives in: `macos` (the Swift
app in `NotchBuddy/`), `windows` (the Tauri app in `windows/`, which also
builds for Linux), `linux` (Linux-only code in that app), `hook` (the Claude
Code relays), `site` (the GitHub Pages site in `docs/`).

**Scope is dropped when a change belongs to no single part**: a fix to the
whole build, to the repository's structure, to tooling, to top-level
documents. Inventing a scope for such a commit is worse than leaving it out:
`build(build):` says nothing, and `build(macos):` outright lies about the
change's boundaries.

```
feat(windows): add a hover delay before the hidden island comes out
fix(hook): exit at once when the app is not running
```

Three rules that matter more than the format:

1. **One commit, one logical change.** If the message body wants the word
   "and," that is two commits.
2. **Every commit builds and passes its tests.** `git bisect` is useless on a
   history where half the commits don't compile.
3. **The body answers "why," not "what."** What was done is visible in the diff.

Messages are written in English.

## Language

Code, names, comments, log and error text, interface labels, commit messages,
`README`, `CONTRIBUTING` and `CHANGELOG` are in English. The code is meant to
be published.

The behaviour specs `docs/SPEC.md` and `docs/INTEGRATIONS.md` are in French;
keep them in French when you update them.

## Workflow

```bash
git switch main && git pull --ff-only
git switch -c feat/short-topic

# ... changes, always with tests ...

git add <the files you changed> && git commit
```

Stage the files you changed by name rather than with `git add -A`, so local
files that are not part of the change stay out of the commit.

Merging into the trunk is `--no-ff` only, so the branch stays visible in
history:

```bash
git switch main
git merge --no-ff feat/short-topic
```

Trunk history is not rewritten. `git push --force` to `main` is forbidden;
`git rebase` is allowed only inside your own not-yet-merged branch.

## Tests

A test is written before the implementation. A test that asserts nothing is
worse than no test: it creates the appearance of coverage.

Tests do not touch the network and do not launch real agent CLIs.

Before a branch is merged, these pass for the part it touches:

```bash
# macOS
bash scripts/test-screen-geometry.sh
bash scripts/test-safe-links.sh

# Windows and Linux app
cd windows
npx tsc --noEmit
cargo check -p coucou
```

## What must not be in the repository

- Secrets, tokens, keys. Never, not even in test data.
- Build output: `windows/target/`, `windows/dist/`, `windows/release/`,
  `node_modules/`.
- Anything from a user's machine: logs, `settings.json` backups, dropped files.

## Pull requests

- One topic per PR, with a short GIF or screenshot for anything visual.
- Build must pass with no new warnings.

## Documents

A behaviour change visible to the user is reflected in `CHANGELOG.md`.
