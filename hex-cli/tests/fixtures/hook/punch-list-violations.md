Here is what I found while poking at the repo — a handful of gaps still
open that we should address before shipping:

1. The inference escalation report needs a `--since` filter so operators
   can scope it to a release window.
2. The dashboard should show a per-tier heatmap, since the current
   bar chart flattens T2 and T2.5 into one column.
3. We still need to wire the subagent-stop hook up for the
   HexFlo task_complete path — right now it only fires on success.
4. Follow-up: the `hex classify` subcommand should take a `--json`
   flag so hooks can consume it without parsing the table.

Separately, here is the status table for the brain daemon subsystems.
None of these are tagged with a task id, so treat them as open:

| Subsystem      | Status   | Notes                                    |
|----------------|----------|------------------------------------------|
| inbox watcher  | pending  | Needs RFC-3339 timestamps on ack events  |
| autoscaler     | broken   | Backs off too aggressively under load    |
| reconcile loop | pending  | Ignores worktree-local edits             |

Both of the enumerations above violate the workflow discipline: the
numbered list has four gap items with no task ids or draft paths, and
the status table enumerates four pending/broken subsystems without any
routing references.
