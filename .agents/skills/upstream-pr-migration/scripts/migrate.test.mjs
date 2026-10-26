import assert from 'node:assert/strict';
import { test } from 'node:test';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtemp, writeFile, readFile, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('./migrate.mjs', import.meta.url));
const realGit = execFileSync('which', ['git'], { encoding: 'utf8' }).trim();
const identity = {
  ...process.env,
  GIT_AUTHOR_NAME: 'Upstream Author',
  GIT_AUTHOR_EMAIL: 'upstream@example.test',
  GIT_COMMITTER_NAME: 'Port Maintainer',
  GIT_COMMITTER_EMAIL: 'maintainer@example.test',
};

test('preserve upstream commits, tested content, and stop on unsafe state', async t => {
  const root = await mkdtemp(join(tmpdir(), 'upstream port test '));
  t.after(() => rm(root, { recursive: true, force: true }));
  const repo = join(root, 'repo');
  await mkdir(repo);
  const git = (...args) => execFileSync(realGit, ['-C', repo, ...args], { encoding: 'utf8', env: identity }).trim();
  git('init', '-q', '--initial-branch=master');
  await writeFile(join(repo, 'base.txt'), 'base\n');
  git('add', '.');
  git('commit', '-qm', 'common base');
  const common = git('rev-parse', 'HEAD');
  git('switch', '-qc', 'upstream');
  await writeFile(join(repo, 'feature.txt'), 'first\n');
  git('add', '.');
  git('commit', '-qm', 'original upstream implementation');
  const first = git('rev-parse', 'HEAD');
  await writeFile(join(repo, 'feature.txt'), 'finished\n');
  git('commit', '-qam', 'original upstream regression fix');
  const headSha = git('rev-parse', 'HEAD');
  git('switch', '-qc', 'target', common);
  await writeFile(join(repo, 'fork.txt'), 'fork package configuration\n');
  git('add', '.');
  git('commit', '-qm', 'target fork configuration');
  const baseSha = git('rev-parse', 'HEAD');
  git('switch', '-qc', 'tested');
  await writeFile(join(repo, 'feature.txt'), 'finished\n');
  git('add', '.');
  git('commit', '-qm', 'tested port');
  git('remote', 'add', 'origin', 'git@github.com:rstackjs/lightningcss.git');
  const plan = join(root, 'plan.json');
  await writeFile(plan, JSON.stringify({
    version: 1, source: 'parcel-bundler/lightningcss', target: 'rstackjs/lightningcss',
    targetBranch: 'master', baseSha,
    pulls: [{ number: 42, headSha, url: 'https://github.com/parcel-bundler/lightningcss/pull/42', portBranch: 'codex/port-upstream-42' }],
  }));
  const run = (command, extra = [], env = identity) => spawnSync(process.execPath,
    [script, command, '--plan', plan, '--number', '42', '--checkout', repo, ...extra],
    { encoding: 'utf8', env });
  const preserve = run('preserve', ['--tree-from', 'tested', '--branch', 'codex/preserved']);
  assert.equal(preserve.status, 0, preserve.stderr);
  for (const sha of [first, headSha, baseSha]) git('merge-base', '--is-ancestor', sha, 'codex/preserved');
  assert.equal(git('rev-parse', 'tested^{tree}'), git('rev-parse', 'codex/preserved^{tree}'));
  assert.equal(git('show', `${headSha}:feature.txt`), 'finished');
  assert.equal(git('show', 'codex/preserved:fork.txt'), 'fork package configuration');
  assert.equal(git('show', '-s', '--format=%an', first), 'Upstream Author');
  const original = git('rev-parse', 'codex/preserved');
  assert.notEqual(run('preserve', ['--tree-from', 'tested', '--branch', 'codex/preserved']).status, 0);
  assert.equal(git('rev-parse', 'codex/preserved'), original);
  assert.equal(run('verify', ['--branch', 'codex/preserved', '--tree-from', 'tested']).status, 0);
  assert.notEqual(run('verify', ['--branch', 'codex/preserved', '--tree-from', 'target']).status, 0);
  assert.notEqual(run('preserve', ['--tree-from', 'upstream', '--branch', 'codex/wrong-base']).status, 0);

  // Redirect only fetches to the fixture so prepare never contacts GitHub.
  const bin = join(root, 'bin');
  await mkdir(bin);
  await writeFile(join(bin, 'git'), `#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const args = process.argv.slice(2);
const index = args.indexOf('fetch');
if (index >= 0) {
  if (args[index + 1] === 'origin') args[index + 1] = ${JSON.stringify(repo)};
  else if (args[index + 1] === 'https://github.com/parcel-bundler/lightningcss.git') {
    args[index + 1] = ${JSON.stringify(repo)};
    args[index + 2] = 'refs/heads/upstream';
  } else throw new Error('Unexpected fetch');
}
const result = spawnSync(${JSON.stringify(realGit)}, args, { stdio: 'inherit' });
process.exit(result.status ?? 1);
`, { mode: 0o755 });
  const env = { ...identity, PATH: `${bin}:${identity.PATH}` };
  const destination = join(root, 'prepared checkout');
  const prepared = run('prepare', ['--dest', destination], env);
  assert.equal(prepared.status, 0, prepared.stderr);
  assert.equal(execFileSync(realGit, ['-C', destination, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), headSha);
  assert.equal(execFileSync(realGit, ['-C', destination, 'rev-parse', 'MERGE_HEAD'], { encoding: 'utf8' }).trim(), baseSha);
  assert.equal(await readFile(join(destination, 'fork.txt'), 'utf8'), 'fork package configuration\n');
  assert.notEqual(run('prepare', ['--dest', destination], env).status, 0);

  const planData = JSON.parse(await readFile(plan, 'utf8'));
  planData.pulls[0].headSha = first;
  await writeFile(plan, JSON.stringify(planData));
  const moved = run('prepare', ['--dest', join(root, 'moved-head')], env);
  assert.notEqual(moved.status, 0);
  assert.match(moved.stderr, /Upstream head moved/);

  git('switch', '-qc', 'conflicting-target', baseSha);
  await writeFile(join(repo, 'feature.txt'), 'target-specific implementation\n');
  git('add', '.');
  git('commit', '-qm', 'conflicting fork API');
  planData.baseSha = git('rev-parse', 'HEAD');
  planData.pulls[0].headSha = headSha;
  await writeFile(plan, JSON.stringify(planData));
  const conflictDir = join(root, 'conflicting checkout');
  const conflict = run('prepare', ['--dest', conflictDir], env);
  assert.notEqual(conflict.status, 0);
  assert.match(conflict.stderr, /Merge stopped/);
  assert.equal(execFileSync(realGit, ['-C', conflictDir, 'diff', '--name-only', '--diff-filter=U'], { encoding: 'utf8' }).trim(), 'feature.txt');
  assert.equal(execFileSync(realGit, ['-C', conflictDir, 'rev-parse', 'MERGE_HEAD'], { encoding: 'utf8' }).trim(), planData.baseSha);
  await writeFile(join(repo, 'dirty.txt'), 'do not overwrite\n');
  const dirty = run('preserve', ['--tree-from', 'tested', '--branch', 'codex/dirty']);
  assert.notEqual(dirty.status, 0);
  assert.match(dirty.stderr, /Commit or save local changes/);
});

test('discover paginates, matches authors, sorts PRs, and flags duplicates', async t => {
  const root = await mkdtemp(join(tmpdir(), 'upstream discovery test '));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = 'parcel-bundler/lightningcss';
  const target = 'rstackjs/lightningcss';
  const url = number => `https://github.com/${source}/pull/${number}`;
  const detail = number => ({
    number, title: `fix ${number}`, html_url: url(number), commits: 2,
    user: { login: number === 42 ? 'AnotherAuthor' : 'SyMind' },
    state: number === 42 ? 'closed' : 'open', merged: number === 42,
    head: { sha: 'b'.repeat(40), ref: `source-${number}` },
  });
  const responses = {
    [`repos/${target}`]: { default_branch: 'main' },
    [`repos/${target}/commits/main`]: { sha: 'a'.repeat(40) },
    [`repos/${source}/pulls?state=open&per_page=100&page=1`]: Array.from({ length: 100 }, (_, i) => ({
      number: i + 1, user: { login: i === 99 ? 'SyMind' : 'OtherAuthor' },
    })),
    [`repos/${source}/pulls?state=open&per_page=100&page=2`]: [{ number: 7, user: { login: 'symind' } }],
    [`repos/${target}/pulls?state=open&per_page=100&page=1`]: [{ title: 'ported change', body: url(7), html_url: `https://github.com/${target}/pull/50` }],
    [`repos/${source}/pulls/7`]: detail(7),
    [`repos/${source}/pulls/100`]: detail(100),
    [`repos/${source}/pulls/42`]: detail(42),
  };
  const fixtures = join(root, 'responses.json');
  await writeFile(fixtures, JSON.stringify(responses));
  const bin = join(root, 'bin');
  await mkdir(bin);
  await writeFile(join(bin, 'gh'), `#!/usr/bin/env node
const { readFileSync } = require('node:fs');
const args = process.argv.slice(2);
if (args[0] === '--version') {
  console.log('gh version 2.0.0 (fixture)');
  process.exit(0);
}
if (args[0] !== 'api' || args[1] !== '--method' || args[2] !== 'GET') throw new Error('Only read-only API calls are allowed');
const responses = JSON.parse(readFileSync(${JSON.stringify(fixtures)}, 'utf8'));
const response = responses[args[3]];
if (!response) throw new Error('Unexpected endpoint: ' + args[3]);
console.log(JSON.stringify(response));
`, { mode: 0o755 });
  const out = join(root, 'plan.json');
  const run = (extraEnv = {}, output = out, selection = ['--author', 'SyMind']) => spawnSync(process.execPath,
    [script, 'discover', '--source', source, '--target', target, ...selection, '--out', output],
    { encoding: 'utf8', env: { ...identity, GH_BIN: join(bin, 'gh'), ...extraEnv } });
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  const plan = JSON.parse(await readFile(out, 'utf8'));
  assert.equal(plan.targetBranch, 'main');
  assert.equal(plan.baseSha, 'a'.repeat(40));
  assert.deepEqual(plan.pulls.map(p => p.number), [7, 100]);
  assert.deepEqual(plan.pulls[0].possibleDuplicates, [`https://github.com/${target}/pull/50`]);
  assert.deepEqual(plan.pulls[1].possibleDuplicates, []);
  assert.notEqual(run().status, 0, 'Do not overwrite a pinned plan');
  const wrongGh = join(bin, 'host-query-gh');
  await writeFile(wrongGh, '#!/usr/bin/env node\nconsole.log("gh, internal host query tool");\n', { mode: 0o755 });
  const wrong = run({ GH_BIN: wrongGh }, join(root, 'wrong-cli-plan.json'));
  assert.notEqual(wrong.status, 0);
  assert.match(wrong.stderr, /is not the GitHub CLI/);

  // Explicit selections must not list every upstream PR or assume one author.
  delete responses[`repos/${source}/pulls?state=open&per_page=100&page=1`];
  delete responses[`repos/${source}/pulls?state=open&per_page=100&page=2`];
  await writeFile(fixtures, JSON.stringify(responses));
  const manualOut = join(root, 'manual.json');
  const manual = run({}, manualOut, ['--pr', '#42,7', '--pr', url(100), '--pr', '42']);
  assert.equal(manual.status, 0, manual.stderr);
  const manualPlan = JSON.parse(await readFile(manualOut, 'utf8'));
  assert.equal(manualPlan.author, null);
  assert.deepEqual(manualPlan.selection, { prs: [7, 42, 100] });
  assert.deepEqual(manualPlan.pulls.map(p => p.number), [7, 42, 100]);
  assert.deepEqual(manualPlan.pulls.map(p => p.author), ['SyMind', 'AnotherAuthor', 'SyMind']);
  assert.equal(manualPlan.pulls[1].merged, true);
  for (const selection of [
    [], ['--author', 'SyMind', '--pr', '7'], ['--pr', '0'], ['--pr', '7,'],
    ['--pr', 'https://github.com/other/repo/pull/7'],
  ]) {
    const invalid = run({}, join(root, 'invalid.json'), selection);
    assert.notEqual(invalid.status, 0, JSON.stringify(selection));
  }
});
