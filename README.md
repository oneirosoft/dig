# dig

[![CI Status](https://github.com/oneirosoft/dig/actions/workflows/ci.yml/badge.svg)](https://github.com/oneirosoft/dig/actions)
[![Latest Release](https://img.shields.io/github/v/release/oneirosoft/dig)](https://github.com/oneirosoft/dig/releases)
[![License: GPL-3.0](https://img.shields.io/github/license/oneirosoft/dig)](https://github.com/oneirosoft/dig/blob/main/LICENSE)
[![Total Downloads](https://img.shields.io/github/downloads/oneirosoft/dig/total)](https://github.com/oneirosoft/dig/releases)
[![Rust Version](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org/)

`dig` is a Git wrapper for stacked PR workflows. It helps you build feature branches on top of other feature branches, keep those parent/child relationships explicit, and see the stack as a tree instead of managing it by memory or convention.

The goal is to make stacked changes easier to review and merge by reducing manual rebases, branch bookkeeping, and the cleanup work that usually follows when parent branches move.

## Quick Start

### Build

`dig` is a Rust CLI. Build it with Cargo:

```bash
cargo build
cargo run -- --help
```

If you want a `dig` binary on your `PATH`:

```bash
cargo install --path .
```

If you do not install it, replace `dig ...` in the examples below with `cargo run -- ...`.

### Initialize dig

Run `dig init` in the repository you want to manage:

```bash
dig init
```

If the current directory is not already a Git repository, `dig init` will create one first. The branch you initialize on becomes dig's trunk branch, which is usually `main`.

### Create a stack

Create a tracked child branch from your current branch:

```bash
dig branch feat/auth
```

Make changes, stage them with Git, and commit through `dig`:

```bash
git add .
dig commit -m "feat: auth"
```

Create another branch on top of that work:

```bash
dig branch feat/auth-ui
git add .
dig commit -m "feat: auth ui"
```

Move a tracked branch under a different tracked parent:

```bash
dig reparent feat/auth-ui -p main
```

Inspect the stack at any time:

```bash
dig tree
```

Create or adopt a GitHub pull request for the current tracked branch:

```bash
dig pr --title "feat: auth" --body "Implements authentication." --draft
```

Open the current branch's pull request in the browser:

```bash
dig pr --view
```

List tracked open pull requests in stack order:

```bash
dig pr list
dig pr list --view
```

### Common commands

```bash
dig init                        # initialize dig in the current directory
dig branch <name>               # create a tracked branch from the current branch
dig branch <name> -p <parent>   # create a tracked branch under a specific parent
dig tree                        # show the full tracked branch tree
dig tree --branch <branch>      # show one branch and its descendants
dig commit -m "message"         # commit and restack tracked descendants if needed
dig pr                          # create or adopt a GitHub PR for the current tracked branch
dig pr --title "title" --body "body" --draft
dig pr --view                   # open the current branch PR in the browser
dig pr list                     # list open GitHub PRs that dig is tracking
dig pr list --view              # list tracked PRs, then open them in the browser
dig sync                        # reconcile local stacks with Git and GitHub, restack, and update remotes
dig sync --continue             # continue a paused restack after resolving conflicts
dig merge <branch>              # merge a tracked branch into its tracked parent
dig clean                       # stop tracking missing local branches and remove merged tracked branches
dig adopt <branch> -p <parent>  # start tracking an existing local branch
dig reparent -p <parent>        # reparent the current tracked branch onto a new parent
dig reparent <branch> -p <parent> # reparent a named tracked branch onto a new parent
dig orphan <branch>             # stop tracking a branch but keep the local branch
```

### Sync stacks

Run `dig sync` to reconcile your local branches, dig's tracked stack metadata, and GitHub pull requests:

```bash
dig sync
```

`dig sync` is the primary command for keeping your entire workspace up to date. It will:

1. **Fetch remotes:** Update local tracking branches from their remotes.
2. **Reconcile state:** Identify branches that were deleted locally or merged on GitHub.
3. **Repair PRs:** Reopen and retarget child pull requests if their parent branch was merged and deleted.
4. **Restack:** Automatically restack tracked branches whose parent branch has moved ahead.
5. **Update GitHub:** Retarget open pull requests if their base branch changed during restacking.
6. **Push updates:** Prompt to push or force-push restacked branches to their remotes.
7. **Cleanup:** Offer to delete tracked branches that are already merged or missing locally.

If `dig` hits a rebase conflict during restacking, it pauses and provides guidance on how to resolve and continue.

### Track GitHub pull requests

`dig pr` uses the GitHub CLI (`gh`) to create a pull request for the current tracked branch, or to adopt the existing open pull request for that branch if one already exists on GitHub.

By default, dig targets the branch's tracked parent as the PR base. Root branches target trunk, child branches target their tracked parent branch, and the tracked PR number is stored locally in `.git/dig/state.json`.

If the branch is not pushed to a resolvable remote yet, `dig pr` prompts before running `git push -u <remote> <branch>` and then continues with PR creation if you confirm.

When dig creates a pull request, it prints both the creation summary and the GitHub link. If you pass
`--title` without `--body`, dig reuses the title as the PR body.

`dig tree` annotates tracked branches that have a PR with `(#123)`.

`dig pr --view` opens the current branch's pull request in the browser. If you combine `--view` with a mutating PR command, dig opens the browser after the command completes.

`dig pr list` shows only open pull requests that are both open on GitHub and currently tracked by dig, rendered in dig's stack order. Each line includes `#<number>: <title>` and the GitHub URL.

### Resolve paused commands

Some commands, including `dig commit`, `dig adopt`, `dig reparent`, `dig merge`, `dig clean`, `dig orphan`, and `dig sync`, may pause if `dig` hits a rebase conflict while restacking tracked descendants.

When that happens:

1. Inspect the conflict state.
2. Edit the conflicted files until the conflict markers are resolved.
3. Stage the resolved files with Git.
4. Resume the paused operation with `dig sync --continue`.

```bash
git status
$EDITOR <conflicted-files>
git add <resolved-files>
dig sync --continue
```

If the next descendant also conflicts, repeat the same process and run `dig sync --continue` again.

While an operation is paused, start by finishing or aborting that rebase before running more `dig` workflow commands. If you abort with `git rebase --abort`, rerun the original `dig` command after the rebase state has been cleared.

## License

`dig` is licensed under the GNU General Public License, version 3 or, at your option, any later version. See [LICENSE](LICENSE) for the full text.

Copyright (C) 2026 Mark Pro. See [COPYRIGHT](COPYRIGHT) for the project copyright notice.

Commercial use of `dig` is allowed. You may use `dig` in commercial environments, on private repositories, and on proprietary codebases.

Using `dig` as a tool against a repository does not by itself change the license of that repository or require that repository to be open source. In other words, running `dig` on your project does not impose the GPL on your project's source code merely because `dig` was used as part of the workflow.

If you modify and redistribute `dig` itself, or distribute a larger combined work that incorporates `dig`'s GPL-covered code, those distributions must comply with the GPL.
