# dagger

[![CI Status](https://github.com/oneirosoft/dagger/actions/workflows/ci.yml/badge.svg)](https://github.com/oneirosoft/dagger/actions)
[![Latest Release](https://img.shields.io/github/v/release/oneirosoft/dagger)](https://github.com/oneirosoft/dagger/releases)
[![License: GPL-3.0](https://img.shields.io/github/license/oneirosoft/dagger)](https://github.com/oneirosoft/dagger/blob/main/LICENSE)
[![Total Downloads](https://img.shields.io/github/downloads/oneirosoft/dagger/total)](https://github.com/oneirosoft/dagger/releases)
[![Rust Version](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org/)

`dagger` is a Git wrapper for stacked PR workflows. It helps you build feature branches on top of other feature branches, keep those parent/child relationships explicit, and see the stack as a tree instead of managing it by memory or convention.

The goal is to make stacked changes easier to review and merge by reducing manual rebases, branch bookkeeping, and the cleanup work that usually follows when parent branches move.

## Quick Start

### Build

`dagger` is a Rust CLI. Build it with Cargo:

```bash
cargo build
cargo run -- --help
```

If you want a `dgr` binary on your `PATH`:

```bash
cargo install --path .
```

If you do not install it, replace `dgr ...` in the examples below with `cargo run -- ...`.

### Initialize dagger

Run `dgr init` in the repository you want to manage:

```bash
dgr init
```

If the current directory is not already a Git repository, `dgr init` will create one first. The branch you initialize on becomes dagger's trunk branch, which is usually `main`.

### Create a stack

Create a tracked child branch from your current branch:

```bash
dgr branch feat/auth
```

Make changes, stage them with Git, and commit through `dgr`:

```bash
git add .
dgr commit -m "feat: auth"
```

Create another branch on top of that work:

```bash
dgr branch feat/auth-ui
git add .
dgr commit -m "feat: auth ui"
```

Move a tracked branch under a different tracked parent:

```bash
dgr reparent feat/auth-ui -p main
```

Inspect the stack at any time:

```bash
dgr tree
```

Create or adopt a GitHub pull request for the current tracked branch:

```bash
dgr pr --title "feat: auth" --body "Implements authentication." --draft
```

Open the current branch's pull request in the browser:

```bash
dgr pr --view
```

List tracked open pull requests in stack order:

```bash
dgr pr list
dgr pr list --view
```

### Common commands

```bash
dgr init                        # initialize dagger in the current directory
dgr branch <name>               # create a tracked branch from the current branch
dgr branch <name> -p <parent>   # create a tracked branch under a specific parent
dgr switch <branch>             # switch directly to a local branch
dgr switch                      # choose a tracked branch from the interactive tree
dgr tree                        # show the full tracked branch tree
dgr tree --branch <branch>      # show one branch and its descendants
dgr commit -m "message"         # commit and restack tracked descendants if needed
dgr pr                          # create or adopt a GitHub PR for the current tracked branch
dgr pr --title "title" --body "body" --draft
dgr pr --view                   # open the current branch PR in the browser
dgr pr list                     # list open GitHub PRs that dagger is tracking
dgr pr list --view              # list tracked PRs, then open them in the browser
dgr sync                        # reconcile local stacks with Git and GitHub, restack, and update remotes
dgr sync --continue             # continue a paused restack after resolving conflicts
dgr merge <branch>              # merge a tracked branch into its tracked parent
dgr clean                       # stop tracking missing local branches and remove merged tracked branches
dgr adopt <branch> -p <parent>  # start tracking an existing local branch
dgr reparent -p <parent>        # reparent the current tracked branch onto a new parent
dgr reparent <branch> -p <parent> # reparent a named tracked branch onto a new parent
dgr orphan <branch>             # stop tracking a branch but keep the local branch
```

When you run `dgr switch` without a branch name, dagger opens an inline tree picker for the tracked stack. Use the arrow keys or `j`/`k` to move, `Enter` to switch, and `Esc` or `q` to cancel.

### Sync stacks

Run `dgr sync` to reconcile your local branches, dagger's tracked stack metadata, and GitHub pull requests:

```bash
dgr sync
```

`dgr sync` is the primary command for keeping your entire workspace up to date. It will:

1. **Fetch remotes:** Update local tracking branches from their remotes.
2. **Reconcile state:** Identify branches that were deleted locally or merged on GitHub.
3. **Repair PRs:** Reopen and retarget child pull requests if their parent branch was merged and deleted.
4. **Restack:** Automatically restack tracked branches whose parent branch has moved ahead.
5. **Update GitHub:** Retarget open pull requests if their base branch changed during restacking.
6. **Push updates:** Prompt to push or force-push restacked branches to their remotes.
7. **Cleanup:** Offer to delete tracked branches that are already merged or missing locally.

If `dgr` hits a rebase conflict during restacking, it pauses and provides guidance on how to resolve and continue.

### Track GitHub pull requests

`dgr pr` uses the GitHub CLI (`gh`) to create a pull request for the current tracked branch, or to adopt the existing open pull request for that branch if one already exists on GitHub.

By default, dagger targets the branch's tracked parent as the PR base. Root branches target trunk, child branches target their tracked parent branch, and the tracked PR number is stored locally in `.git/.dagger/state.json`.

If the branch is not pushed to a resolvable remote yet, `dgr pr` prompts before running `git push -u <remote> <branch>` and then continues with PR creation if you confirm.

When dagger creates a pull request, it prints both the creation summary and the GitHub link. If you pass
`--title` without `--body`, dagger reuses the title as the PR body.

`dgr tree` annotates tracked branches that have a PR with `(#123)`.

`dgr pr --view` opens the current branch's pull request in the browser. If you combine `--view` with a mutating PR command, dagger opens the browser after the command completes.

`dgr pr list` shows only open pull requests that are both open on GitHub and currently tracked by dagger, rendered in dagger's stack order. Each line includes `#<number>: <title>` and the GitHub URL.

### Resolve paused commands

Some commands, including `dgr commit`, `dgr adopt`, `dgr reparent`, `dgr merge`, `dgr clean`, `dgr orphan`, and `dgr sync`, may pause if `dagger` hits a rebase conflict while restacking tracked descendants.

When that happens:

1. Inspect the conflict state.
2. Edit the conflicted files until the conflict markers are resolved.
3. Stage the resolved files with Git.
4. Resume the paused operation with `dgr sync --continue`.

```bash
git status
$EDITOR <conflicted-files>
git add <resolved-files>
dgr sync --continue
```

If the next descendant also conflicts, repeat the same process and run `dgr sync --continue` again.

While an operation is paused, start by finishing or aborting that rebase before running more `dgr` workflow commands. If you abort with `git rebase --abort`, rerun the original `dgr` command after the rebase state has been cleared.

## License

`dagger` is licensed under the GNU General Public License, version 3 or, at your option, any later version. See [LICENSE](LICENSE) for the full text.

Copyright (C) 2026 Mark Pro. See [COPYRIGHT](COPYRIGHT) for the project copyright notice.

Commercial use of `dagger` is allowed. You may use `dagger` in commercial environments, on private repositories, and on proprietary codebases.

Using `dagger` as a tool against a repository does not by itself change the license of that repository or require that repository to be open source. In other words, running `dagger` on your project does not impose the GPL on your project's source code merely because `dagger` was used as part of the workflow.

If you modify and redistribute `dagger` itself, or distribute a larger combined work that incorporates `dagger`'s GPL-covered code, those distributions must comply with the GPL.
