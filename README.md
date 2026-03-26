# dig

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

Inspect the stack at any time:

```bash
dig tree
```

### Common commands

```bash
dig init                        # initialize dig in the current directory
dig branch <name>               # create a tracked branch from the current branch
dig branch <name> -p <parent>   # create a tracked branch under a specific parent
dig tree                        # show the full tracked branch tree
dig tree --branch <branch>      # show one branch and its descendants
dig commit -m "message"         # commit and restack tracked descendants if needed
dig sync                        # reconcile local dig state, restack stale stacks, then offer cleanup
dig sync --continue             # continue a paused restack after resolving conflicts
dig merge <branch>              # merge a tracked branch into its tracked parent
dig clean                       # remove tracked branches already merged into their parent
dig adopt <branch> -p <parent>  # start tracking an existing local branch
dig orphan <branch>             # stop tracking a branch but keep the local branch
```

### Sync local stacks

Run `dig sync` when local Git state and dig's tracked stack metadata may have drifted apart:

```bash
dig sync
```

Today `dig sync` is local-only. It will:

1. Stop tracking branches that were deleted locally but are still tracked by dig.
2. Restack tracked branches whose parent branch has moved ahead.
3. Offer the existing cleanup flow for tracked branches already merged into their parent.

If cleanup finds merged branches, `dig sync` reuses the same delete prompt as `dig clean`. If you decline that prompt, sync still succeeds and leaves cleanup for later.

Remote sync is intentionally out of scope for now. Future GitHub and `gh` integration can extend `dig sync`, but the current command only reconciles local branches and local dig state.

### Resolve paused commands

Some commands, including `dig commit`, `dig adopt`, `dig merge`, `dig clean`, `dig orphan`, and `dig sync`, may pause if `dig` hits a rebase conflict while restacking tracked descendants.

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
