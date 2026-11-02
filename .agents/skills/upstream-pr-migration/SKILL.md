---
name: upstream-pr-migration
description: Port upstream lightningcss pull requests into rstackjs/lightningcss, preserving original commits, adapting fork interfaces, validating changes, and merging in order when requested.
---

# Upstream PR migration

Use this workflow for moving upstream lightningcss PRs into the Rstack fork. Creating ports does not authorize merging them; merge only when requested. Use GitHub CLI (`gh api`, `gh pr`) for all GitHub operations. Keep zx for orchestration and local Git operations. This workflow uses gh CLI at the user's request; honor current session instructions and authorization for write operations.

## Discover and pin the inputs

Read repository instructions and check the worktree, remotes, default branch, existing target PRs, and merge policy. This repository's default branch is `master`; resolve the actual default rather than assuming `main`. Push to `origin` (`rstackjs/lightningcss`), not the contributor's fork.

The scripts require Node 20+, pnpm, Git, and an authenticated [GitHub CLI](https://cli.github.com/). Confirm `gh --version` identifies GitHub CLI and run `gh auth status`. A different tool can also be named `gh`; the helper rejects it. Set `GH_BIN=/absolute/path/to/github-cli/gh` when needed, and use that executable for the manual commands below too. For a repeatable local snapshot, install the workspace package with `pnpm --filter @rstack/skill-upstream-pr-migration install --ignore-scripts`, then run from the repository root:

```sh
pnpm --filter @rstack/skill-upstream-pr-migration migrate discover \
  --source parcel-bundler/lightningcss --target rstackjs/lightningcss \
  --author SyMind --out /tmp/lightningcss-ports.json
```

`--author LOGIN` selects that person's currently open upstream PRs; the author is supplied by the caller, not fixed to the example. Alternatively, specify PRs directly, including PRs from different authors:

```sh
pnpm --filter @rstack/skill-upstream-pr-migration migrate discover \
  --source parcel-bundler/lightningcss --target rstackjs/lightningcss \
  --pr 1333,1334 --pr https://github.com/parcel-bundler/lightningcss/pull/1338 \
  --out /tmp/lightningcss-ports.json
```

Choose exactly one mode: `--author` or `--pr`. `--pr` accepts repeated options, comma-separated numbers, and GitHub PR URLs belonging to `--source`. Explicit selections may include closed or merged upstream PRs; inspect their state and whether the target already contains their changes. The helper deduplicates selections, sorts by PR number, pins the target base and each upstream head SHA, records each PR's actual author, and flags possible duplicates by title or source URL. Author discovery paginates all open upstream PRs; explicit selection fetches only the requested upstream PRs. Inspect duplicate matches before changing the plan. See [scripts/migrate.mjs](scripts/migrate.mjs) for the snapshot schema. Keep machine-specific plans, checkouts, and logs outside the repository.

## Preserve history and prepare each port

```text
Upstream PR head (original commits and authors)
              +
Pinned target default-branch commit
              ↓
Isolated checkout: merge target into upstream head
              ↓
Resolve conflicts + adapt interfaces + retain both sets of tests
              ↓
Commit → test → verify ancestry and tree → push origin → create PR
```

Use a separate branch per PR. Keep original commit SHAs by starting at the upstream head and adding a merge/adaptation commit. Cherry-picking preserves authorship but changes commit identities; squashing the source patch also loses its individual commits.

```sh
pnpm --filter @rstack/skill-upstream-pr-migration migrate prepare \
  --plan /tmp/lightningcss-ports.json --number 1333 \
  --checkout /absolute/path/to/lightningcss --dest /tmp/lightningcss-port-1333
```

The source checkout supplies objects, not uncommitted files. The helper checks `origin`, fetches the pinned base and upstream PR, rejects a moved upstream head or reused destination, and leaves the merge uncommitted. Inspect the result and commit it. On conflict, it stops with the isolated checkout intact. Resolve ordinary merge conflicts there; if a pull or push reports a conflict or rejected update, stop and ask the user as required by the repository rules. Do not rebase, amend, force-push, or automatically retry mutations.

Prefer copying complete upstream test files, then adapt only project-specific configuration and APIs. Preserve the fork's crate names, version, release configuration, current upstream fixes, and existing tests. Concurrent test insertions should retain both groups.

If a port was already assembled and tested on the target base using patch/cherry-pick scripts, reconstruct its history without changing that tested tree:

```sh
pnpm --filter @rstack/skill-upstream-pr-migration migrate preserve \
  --plan /tmp/lightningcss-ports.json --number 1333 \
  --checkout /tmp/lightningcss-port-1333 \
  --tree-from tested-port-branch --branch codex/preserve-upstream-1333
```

This implements the previous migration's `git commit-tree` script: use the tested tree verbatim, with the upstream PR head and pinned target base as parents. It refuses dirty checkouts, existing output branches, and trees without the target base in their ancestry. This command does **not** run tests or prove the patch is semantically correct; inspect the resulting diff. The original upstream head must remain an ancestor and the final tree must equal the tested tree.

## Validate and publish

In the prepared checkout, run checks appropriate to the change. For the Rust ports in this migration:

```sh
cargo test --all-features --locked
cargo fmt --all -- --check
git diff --check
```

Use the user's definition of “all tests” when requested: Rust tests, unit tests, and clippy. Node/WASM binding or browser comparisons are useful when affected; only claim checks actually run on this port. A previous PR's validation is evidence, not a new local result.

Run the integrity helper after committing, before pushing:

```sh
pnpm --filter @rstack/skill-upstream-pr-migration migrate verify \
  --plan /tmp/lightningcss-ports.json --number 1333 \
  --checkout /tmp/lightningcss-port-1333
```

For a reconstructed branch, also pass `--branch codex/preserve-upstream-1333 --tree-from tested-port-branch`. Re-run tests if the source tree changes after validation. Never test one tree and publish another.

Push the final branch directly to `origin`. Create the PR with `gh pr create` against the discovered default branch. Write its exact description to a file and pass `--body-file`; include the upstream link, concrete before/after example, necessary adaptations, local checks, and material limitations. Append this signature to every new GitHub PR body, issue/PR comment, or review:

```markdown
---
_by OpenAI Codex_
```

From the prepared checkout, substitute the inspected plan values and prepared description:

```sh
git push origin codex/port-upstream-1333
"${GH_BIN:-gh}" pr create --repo rstackjs/lightningcss \
  --base master --head codex/port-upstream-1333 \
  --title 'fix: factor shared custom properties in adjacent rules' \
  --body-file /tmp/lightningcss-pr-1333.md
```

For description updates use `gh pr edit NUMBER --repo OWNER/REPO --body-file FILE`. Inspect existing PRs before retrying a failed creation; a nonzero exit is not proof that no PR was created. The helper commands do not publish anything implicitly.

Attach every created PR to the chat using the app's artifact tool when available. Leave the contributor's original PR open unless explicitly asked otherwise.

## Merge in order only when requested

List **all** currently open target PRs, including pre-existing ones, and sort by PR number. Inspect checks and repository merge methods. Use merge commits when allowed to retain history on the target branch. This fork allowed only squash during the 2026-10 migration: explain that limitation, use squash if consistent with the request, and retain source branches containing the original history. If the user requires original SHAs on the default branch, resolve that policy conflict before merging.

Use `gh api --method GET 'repos/rstackjs/lightningcss/pulls?state=open&per_page=100&page=1'` to enumerate open target PRs, following every page. Inspect policy with `gh api --method GET repos/rstackjs/lightningcss` (`allow_merge_commit`, `allow_squash_merge`, `allow_rebase_merge`). For each PR, fetch its current base/head and CI, then merge with the full 40-character expected head SHA. For example, after choosing squash under the repository policy:

```sh
"${GH_BIN:-gh}" pr view 3 --repo rstackjs/lightningcss \
  --json state,baseRefName,headRefOid,mergeable,mergeStateStatus,statusCheckRollup
"${GH_BIN:-gh}" pr checks 3 --repo rstackjs/lightningcss
# If required checks exist, wait for those checks before merging.
"${GH_BIN:-gh}" pr checks 3 --repo rstackjs/lightningcss --required --watch --fail-fast
"${GH_BIN:-gh}" pr merge 3 --repo rstackjs/lightningcss --squash \
  --match-head-commit FULL_40_CHARACTER_HEAD_SHA
"${GH_BIN:-gh}" pr view 3 --repo rstackjs/lightningcss --json state,mergedAt,mergeCommit
```

Use `--merge` instead of `--squash` when permitted and appropriate; do not use `--rebase`, `--admin`, or `--delete-branch`. Confirm each PR is actually merged before moving to the next: merge queues can enqueue rather than complete the operation. Re-read each following PR against the new base. A previously green or conflict-free PR may now need adaptation. Textually clean merges can also contain incompatible APIs: inspect new direct calls to changed interfaces, not just conflict markers.

For conflicts or interface changes, merge the latest target base into that PR branch, add adaptation commits, validate the combined tree, and push the existing branch. Wait for required checks; do not bypass protections or modify repository merge settings. If checks fail, diagnose before retrying. An allowed merge method's rejection is not permission to change repository policy.

After the last merge, verify that no requested PR remains open and test the resulting default-branch tree. Keep the user's working checkout intact by using the isolated checkouts.

Read [references/lightningcss-2026-10.md](references/lightningcss-2026-10.md) when handling the PrinterTrait/source-map transition, the known bundler test-order failure, or comparing the scripts with the original migration.
