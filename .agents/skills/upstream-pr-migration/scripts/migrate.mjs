#!/usr/bin/env zx
import { $, fs } from 'zx';
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';

$.verbose = false;
const ghBin = process.env.GH_BIN || 'gh';
let checkedGh = false;
const { values: args, positionals } = parseArgs({
  args: process.argv.slice(process.argv.findIndex((value, index) =>
    index > 0 && resolve(value) === fileURLToPath(import.meta.url),
  ) + 1),
  allowPositionals: true,
  options: {
    ...Object.fromEntries(
      ['source', 'target', 'author', 'out', 'plan', 'number', 'checkout', 'dest', 'tree-from', 'branch']
        .map(name => [name, { type: 'string' }]),
    ),
    pr: { type: 'string', multiple: true },
  },
});
const help = `Usage: pnpm --filter @rstack/skill-upstream-pr-migration migrate <command> [options]

discover --source owner/repo --target owner/repo --author login --out plan.json
discover --source owner/repo --target owner/repo --pr 123 --pr 456,789 --out plan.json
prepare  --plan plan.json --number 123 --checkout local-repo --dest new-directory
preserve --plan plan.json --number 123 --checkout local-repo --tree-from tested-ref --branch codex/new-branch
verify   --plan plan.json --number 123 --checkout local-repo [--branch ref] [--tree-from tested-ref]

discover uses read-only gh api calls (GH_BIN overrides the GitHub CLI executable).
Other commands only change local Git data.
prepare leaves the merge uncommitted, including conflicts, for inspection and adaptation.
preserve copies an already-tested tree into a merge commit with the original PR head as a parent.
No command pushes, creates PRs, or merges PRs on GitHub.`;

function required(name) {
  assert(args[name], `Missing --${name}`);
  return args[name];
}

function repository(value) {
  assert(/^[\w.-]+\/[\w.-]+$/.test(value), `Invalid repository: ${value}`);
  return value;
}

function sha(value) {
  assert(/^[0-9a-f]{40}$/.test(value), `Expected full commit SHA: ${value}`);
  return value;
}

function prNumber(value, source) {
  const url = value.match(/^https:\/\/github\.com\/([^/]+\/[^/]+)\/pull\/(\d+)\/?$/);
  if (url) assert.equal(url[1].toLowerCase(), source.toLowerCase(), 'PR URL must belong to --source');
  const text = url ? url[2] : value.replace(/^#/, '');
  assert(/^\d+$/.test(text), `Invalid PR number or URL: ${value}`);
  const number = Number(text);
  assert(Number.isSafeInteger(number) && number > 0, `Invalid PR number: ${value}`);
  return number;
}

async function gh(endpoint) {
  if (!checkedGh) {
    const version = await $`${ghBin} --version`;
    assert(/^gh version \d+\./m.test(version.stdout),
      `${ghBin} is not the GitHub CLI. Install cli.github.com or set GH_BIN to its executable.`);
    checkedGh = true;
  }
  return JSON.parse((await $`${ghBin} api --method GET ${endpoint}`).stdout);
}

async function pages(endpoint) {
  const items = [];
  for (let page = 1; ; page++) {
    const batch = await gh(`${endpoint}${endpoint.includes('?') ? '&' : '?'}per_page=100&page=${page}`);
    assert(Array.isArray(batch), 'Expected a paginated GitHub collection');
    items.push(...batch);
    if (batch.length < 100) return items;
  }
}

async function discover() {
  const source = repository(required('source'));
  const target = repository(required('target'));
  assert(Boolean(args.author) !== Boolean(args.pr), 'Choose exactly one of --author or --pr');
  const author = args.author || null;
  const requested = args.pr
    ? [...new Set(args.pr.flatMap(value => value.split(',')).map(value => prNumber(value.trim(), source)))].sort((a, b) => a - b)
    : null;
  const destination = resolve(required('out'));
  assert(!await fs.pathExists(destination), `Refusing to overwrite ${destination}`);
  const targetRepo = await gh(`repos/${target}`);
  const targetBranch = targetRepo.default_branch;
  const base = await gh(`repos/${target}/commits/${encodeURIComponent(targetBranch)}`);
  const selected = requested || (await pages(`repos/${source}/pulls?state=open`))
    .filter(p => p.user.login.toLowerCase() === author.toLowerCase())
    .map(p => p.number).sort((a, b) => a - b);
  const existing = await pages(`repos/${target}/pulls?state=open`);
  const pulls = [];
  for (const number of selected) {
    const p = await gh(`repos/${source}/pulls/${number}`);
    pulls.push({
      number: p.number,
      title: p.title,
      author: p.user.login,
      state: p.state,
      merged: p.merged,
      url: p.html_url,
      headSha: sha(p.head.sha),
      sourceBranch: p.head.ref,
      portBranch: `codex/port-upstream-${p.number}`,
      commits: p.commits,
      possibleDuplicates: existing.filter(other =>
        other.title === p.title || (other.body || '').includes(p.html_url),
      ).map(other => other.html_url),
    });
  }
  const selection = author ? { author } : { prs: requested };
  const plan = { version: 1, source, target, author, selection, targetBranch, baseSha: sha(base.sha), pulls };
  await fs.outputJson(destination, plan, { spaces: 2, flag: 'wx' });
  console.log(JSON.stringify(plan, null, 2));
}

async function context() {
  const plan = await fs.readJson(resolve(required('plan')));
  assert.equal(plan.version, 1, 'Unsupported plan version');
  repository(plan.source);
  repository(plan.target);
  sha(plan.baseSha);
  const pull = plan.pulls.find(p => String(p.number) === required('number'));
  assert(pull, 'PR is not in the plan');
  sha(pull.headSha);
  assert(Number.isSafeInteger(pull.number) && pull.number > 0, 'Invalid PR number');
  const checkout = resolve(required('checkout'));
  const git = (...words) => $`git -C ${checkout} ${words}`;
  return { plan, pull, checkout, git };
}

async function clean(git) {
  assert.equal((await git('status', '--porcelain')).stdout.trim(), '', 'Commit or save local changes first');
}

async function freshBranch(git, name) {
  assert(name.startsWith('codex/'), 'Use a codex/ branch name');
  await git('check-ref-format', '--branch', name);
  const exists = await git('show-ref', '--verify', '--quiet', `refs/heads/${name}`).nothrow();
  assert.equal(exists.exitCode, 1, `Branch already exists or cannot be checked: ${name}`);
}

async function prepare() {
  const { plan, pull, checkout, git } = await context();
  assert.equal(pull.possibleDuplicates?.length || 0, 0, 'Inspect existing target PRs and update the plan before preparing a duplicate');
  const remote = (await git('remote', 'get-url', 'origin')).stdout.trim();
  const fullName = remote.replace(/\.git$/, '').match(/^(?:git@github\.com:|https:\/\/github\.com\/)([^/]+\/[^/]+)$/)?.[1];
  assert.equal(fullName?.toLowerCase(), plan.target.toLowerCase(), 'origin must point to the target repository');
  const destination = resolve(required('dest'));
  assert(!await fs.pathExists(destination), `Destination already exists: ${destination}`);
  await $`git clone --shared --no-checkout -- ${checkout} ${destination}`;
  const port = (...words) => $`git -C ${destination} ${words}`;
  await port('remote', 'set-url', 'origin', remote);
  await port('fetch', 'origin', plan.baseSha);
  await port('fetch', `https://github.com/${plan.source}.git`, `refs/pull/${pull.number}/head`);
  assert.equal((await port('rev-parse', 'FETCH_HEAD')).stdout.trim(), pull.headSha, 'Upstream head moved: regenerate the plan');
  await freshBranch(port, pull.portBranch);
  await port('switch', '-c', pull.portBranch, pull.headSha);
  const result = await port('merge', '--no-ff', '--no-commit', plan.baseSha).nothrow();
  console.log(`Checkout: ${destination}`);
  if (result.exitCode !== 0) {
    throw new Error(`Merge stopped. Inspect ${destination}, resolve conflicts manually, then commit. No push was attempted.\n${result.stderr}`);
  }
  console.log('Inspect the merge, adapt interfaces and tests, then commit and run validation.');
}

async function verify({ plan, pull, git }, branch, testedRef) {
  await git('merge-base', '--is-ancestor', pull.headSha, branch);
  await git('merge-base', '--is-ancestor', plan.baseSha, branch);
  await git('diff', '--check', `${plan.baseSha}...${branch}`);
  const tree = (await git('rev-parse', `${branch}^{tree}`)).stdout.trim();
  if (testedRef) {
    assert.equal(tree, (await git('rev-parse', `${testedRef}^{tree}`)).stdout.trim(), 'Published tree differs from tested tree');
  }
  console.log(JSON.stringify({ number: pull.number, branch, tree, upstreamHead: pull.headSha, base: plan.baseSha }));
}

async function preserve() {
  const ctx = await context();
  const { plan, pull, git } = ctx;
  await clean(git);
  const testedRef = required('tree-from');
  const branch = required('branch');
  await freshBranch(git, branch);
  // Only use a tree built on the pinned target base, never a raw upstream tree.
  await git('merge-base', '--is-ancestor', plan.baseSha, testedRef);
  await git('diff', '--check', `${plan.baseSha}...${testedRef}`);
  const tree = (await git('rev-parse', `${testedRef}^{tree}`)).stdout.trim();
  const message = `Integrate upstream PR #${pull.number} with ${plan.target}\n\nPreserve original upstream commits from ${pull.url}.\nUse the tested tree from ${testedRef} against ${plan.targetBranch} (${plan.baseSha}).`;
  const commit = (await git('commit-tree', tree, '-p', pull.headSha, '-p', plan.baseSha, '-m', message)).stdout.trim();
  await git('branch', branch, commit);
  await verify(ctx, branch, testedRef);
}

try {
  const command = positionals[0];
  if (!command || command === 'help') console.log(help);
  else if (command === 'discover') await discover();
  else if (command === 'prepare') await prepare();
  else if (command === 'preserve') await preserve();
  else if (command === 'verify') {
    const ctx = await context();
    await clean(ctx.git);
    await verify(ctx, args.branch || 'HEAD', args['tree-from']);
  } else throw new Error(`Unknown command: ${command}\n${help}`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
