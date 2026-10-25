import {execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';

export function ciState(runs, sha) {
  const run = runs.filter(run => run.head_sha === sha && run.head_branch === 'main' && run.event === 'push')
    .sort((a, b) => b.run_number - a.run_number || b.run_attempt - a.run_attempt)[0];
  if (!run || run.status !== 'completed') return 'waiting';
  return run.conclusion === 'success' ? 'success' : 'failure';
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const {GITHUB_REPOSITORY: repo, GITHUB_SHA: sha} = process.env;
  if (!repo || !/^[a-f0-9]{40}$/.test(sha ?? '')) throw new Error('Repository and full commit SHA are required');
  const ref = process.env.GITHUB_REF;
  if (ref && ref !== 'refs/heads/main' && !ref.startsWith('refs/tags/v')) throw new Error('Publishing is limited to main and version tags');
  for (let attempt = 0; attempt < 120; attempt++) {
    const result = JSON.parse(execFileSync('gh', ['api', `repos/${repo}/actions/workflows/ci.yml/runs?head_sha=${sha}&event=push&per_page=100`], {encoding: 'utf8'}));
    const state = ciState(result.workflow_runs, sha);
    if (state === 'success') { console.log(`CI passed for ${sha}`); process.exit(0); }
    if (state === 'failure') throw new Error(`CI failed for ${sha}; publishing is blocked`);
    console.log(`Waiting for main-branch CI at ${sha}`);
    await new Promise(resolve => setTimeout(resolve, 30000));
  }
  throw new Error('No successful CI run for this commit within one hour');
}
