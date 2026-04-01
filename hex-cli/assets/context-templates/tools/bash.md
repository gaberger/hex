Executes a given bash command and returns its output. The working directory persists between commands, but shell state does not.

AVOID using for: cat, head, tail, sed, awk, echo — use dedicated tools instead.

For git commands:
- Prefer creating new commits rather than amending
- Never skip hooks (--no-verify) unless explicitly asked
- Never force-push to main/master

For PRs: use gh CLI for ALL GitHub-related tasks.
