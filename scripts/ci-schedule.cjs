// Scheduled coverage runs once per revision, including revisions that failed.
// Push runs exercise only @core and therefore do not satisfy this check.
module.exports = async function shouldRunCi({ github, context, core }) {
  if (context.eventName !== 'schedule') return true;

  const repo = context.repo;
  const { data: current } = await github.rest.actions.getWorkflowRun({
    ...repo, run_id: context.runId,
  });
  for await (const { data } of github.paginate.iterator(
    github.rest.actions.listWorkflowRuns,
    { ...repo, workflow_id: current.workflow_id, head_sha: context.sha, per_page: 100 },
  )) {
    // Octokit's paginator normalizes workflow_runs into the page's data array.
    const previous = data.find(run => (
      run.id < context.runId
      && run.head_sha === context.sha
      && ['schedule', 'workflow_dispatch'].includes(run.event)
    ));
    if (previous) {
      core.info(`Skipping unchanged revision ${context.sha}; full CI run: ${previous.html_url}`);
      return false;
    }
  }
  return true;
};
