---
name: hex-feedback-loop
description: Give every session a way to check its own work before a human sees it, and regression-test the configuration that steers the agent. Use when the user says "how do I verify this", "the agent keeps shipping broken code", "add tests to the loop", "set up evals", "make it check its own work", or when a task is about to be reported done without evidence.
---

# Hex Feedback Loop — Stage 4, Test

**What changes:** the session checks its own work and fixes its own mistakes
before an engineer sees them. QA stops being a stage boundary and becomes
something woven through implementation.

Traditional: the signal that code works arrives late — CI minutes later, a tester
days later, production weeks later. When an agent produces the code, a late
signal means a person has to check all of its output, and that person becomes the
bottleneck.

## Prerequisites

None. A test suite and a build that each run with one command.

## Part 1 — the loop inside the session

1. **Collapse verification to one command.** If checking the work takes a
   sequence of commands and some environment knowledge, wrap it in one target
   that exits non-zero on failure (`make test`, `npm test`, `hex dev validate`).
2. **List those commands in `CLAUDE.md` with an example of healthy output**, so
   the agent can tell a passing run from a run that merely finished.
3. **State a quantifiable target** so the agent can check itself without asking:
   "all tests in `test_status.py` pass", "the endpoint returns 200 with the new
   field", "the screenshot matches the attached mock".
4. **For bug fixes, write the failing test first.** Ask for the bug reproduced as
   a test, run it, confirm it fails *for the reason you expect*, and commit that
   test. Only then fix the code, without editing the test. A test that existed
   before the fix, and that the agent could not rewrite, is the proof the bug is
   gone.
5. **For UI work, close the loop visually.** Give the agent a browser or
   screenshot tool and the mock, then implement → screenshot → compare → adjust.
   Two or three rounds is normal and each should improve.
6. **Make verification part of "done."** Run the checks before reporting a task
   complete and paste the output.
7. **Protect the loop.** An agent fixing code must not be able to weaken the
   check on that code. A hook that blocks edits to test files during a fix task
   does this; the fallback is rejecting any test-file change in review.

```markdown
## Verifying your work
- Build: make build (must finish with "Build succeeded")
- Test: make test (all green; never skip or delete a failing test)
- Lint: make lint (zero warnings)
Run all three before reporting any task complete, and paste the output.
If a test fails, fix the code, not the test.
```

**The feedback loop is not the verifier subagent.** The loop runs throughout the
task, as many times as the work needs. The verifier subagent is one way to
package the *final* check: a fresh context window, run once the session believes
it is done, so the verdict is not colored by the assumptions that produced the
code.

In a hex project the independent oracle is the behavioral spec written in Stage 2
plus `hex validate`; `hex verify "<claim>"` adversarially checks a specific
claim about the repo and returns CONFIRMED / REFUTED / INCONCLUSIVE.

## Part 2 — evals for the configuration

The configuration that steers the agent — `CLAUDE.md`, skills, hooks, subagent
definitions — deserves the regression testing that code gets. Evals are the
AI-native equivalent of stage-gate QA: a suite that runs whenever that
configuration changes or a model is swapped, and says whether the agent still
does the work to the same standard.

1. Collect 20–50 real tasks from recent work, each with its expected outcome.
2. Write each as an eval: the prompt plus the checks that define acceptable —
   tests pass, lint clean, behavior unchanged, policy followed.
3. Run the suite non-interactively in CI, on a schedule and on any change to
   `CLAUDE.md` or `.claude/**`.
4. Gate configuration changes on the result. A skill edit that drops the pass
   rate gets reviewed before it merges.
5. Give every production incident an eval, written by the team that owned it. It
   stays in the suite as a regression test.

```yaml
name: Agent evals
on:
  pull_request:
    paths: ['CLAUDE.md', '.claude/**']
  schedule:
    - cron: '0 2 * * *'
jobs:
  evals:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: npm install -g @anthropic-ai/claude-code
      - name: Run eval suite
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        run: |
          for eval in evals/*.json; do
            claude -p "$(jq -r '.prompt' $eval)" \
              --allowedTools "Read,Edit,Bash(make test)" \
              --output-format json > result.json
            ./evals/check.sh "$eval" result.json
          done
```

Treat the suite as live. As models improve, cases that once discriminated stop
doing so and new ones have to come in from monitoring.

## Governance

What is enforced: verification before a task is reported done, and the block on
editing test files during a fix — both as hooks where the organization wants them
guaranteed. The evidence is the literal command output, so it comes from the
toolchain rather than from the agent's summary. It is logged in the session
transcript and in the PR's check run. The code owner approves at review, and can
concentrate on intent and risk because the mechanical evidence is attached.

## Measure it

- **Leading:** first-pass CI success rate for agent-written changes; eval pass
  rate over time; how long a production incident takes to become a permanent
  eval.
- **Lagging:** review time per PR, which should fall once the tests catch what
  reviewers used to catch; change failure rate; regressions caught in CI versus
  regressions found in production.

## Next

A change that passed its own checks goes to `hex-review-gate`.
