# Intent: developer skills that follow the AI-native SDLC playbook

Author: gary (operator). Status: accepted.
Source: https://claude.com/blog/the-ai-native-sdlc-playbook

## Problem

hex ships developer skills covering the build half of the lifecycle — scaffold,
generate, workplan, worktree, validate, analyze. The stages either side of build
are not represented as skills at all: there is no artifact for captured intent,
no requirements-and-design pass, no written review policy, no eval suite over the
agent's own configuration, and no deterministic trigger that turns a production
anomaly back into planned work. Those are exactly the human-speed stages that
become the bottleneck once the agent writes most of the diff.

## Proposed outcome

A skill per SDLC stage, woven into the existing hex skills so each stage ends by
committing an artifact the next stage reads, and the chain of commits is the
audit trail.

## Affected users and systems

Every project hex is installed into (skills ship embedded in the CLI and are
extracted on init); the existing hex-workplan, hex-feature-dev and hex-validate
skills, which gain their place in the chain.

## Constraints

Embedded assets must stay project-generic — no references to this repo's internal
crate or module names. Skills may only reference verbs that exist in `hex --help`.

## Open questions

Whether the eval suite (Stage 4, part 2) should be a hex verb rather than a
project-local CI workflow. Deferred until a project actually runs one.
