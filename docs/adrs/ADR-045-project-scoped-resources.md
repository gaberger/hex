# ADR-045: Project-Scoped ADRs, Config Templates, and Embedded Chat

**Status:** Accepted
**Date:** 2025-07-19
**Drivers:** Projects need self-contained ADRs, customizable configs, and integrated chat for inference-driven development.

## Context

The hex-nexus dashboard currently has three gaps in its project management experience:

1. **ADRs are global.** The ADR browser resolves `docs/adrs/` relative to the nexus working directory, not per-project. When managing multiple projects, each project's architectural decisions should live in its own `docs/adrs/` directory.

2. **Project init creates empty config directories.** `POST /api/projects/init` scaffolds `.claude/skills/` and `.claude/agents/` as empty directories. New projects start with no agent definitions, no skills, and no hooks — requiring manual setup before AI-driven development can begin.

3. **Chat is disconnected from project context.** The existing ChatView is a full-page route (`#/project/{id}/chat`) with no project scoping. Developers need a quick chat panel embedded in the project detail view, connected to the inference server, for discussing architecture and getting help without leaving their project context.

## Decision

### 1. Project-Scoped ADR Routes

We will add REST endpoints scoped to projects:

- `GET /api/projects/{id}/adrs` — list ADRs from the project's `docs/adrs/` directory
- `GET /api/projects/{id}/adrs/{adr_id}` — read a specific ADR
- `PUT /api/projects/{id}/adrs/{adr_id}` — save/update an ADR

The existing global `/api/adrs` endpoint remains as fallback for the hex-intf repo itself. Core ADR parsing logic is extracted into shared functions (`list_adrs_from_dir`, `get_adr_from_dir`) reused by both global and project-scoped handlers.

The frontend ADRBrowser branches its fetch URL based on whether `projectId` is present in the route.

### 2. Config/Skills/Agents Copy at Project Init

We will embed default agent definitions (`agents/*.yml`) and skill definitions (`skills/*/SKILL.md`) into the hex-nexus binary via `rust-embed`. On `POST /api/projects/init`:

- Copy all agent YAML files to `{project}/.claude/agents/`
- Copy all skill directories to `{project}/.claude/skills/`
- Write a default `.claude/settings.json` with standard hooks
- Write a default `.hex/project.yaml` manifest

All writes are **idempotent** — existing files are never overwritten. This allows per-project customization after initial scaffolding.

### 3. Embedded Project Chat Widget

We will create a compact, toggleable chat panel within the ProjectDetail view:

- A factory function `createProjectChat(projectId)` returns project-scoped chat signals
- WebSocket connects to `/ws/chat?project_id={id}` (already supported by the backend)
- The widget reuses the existing `MessageList` component for message rendering
- Toggle via a floating chat button; slides in as a right-side panel (~350px)

This is separate from the full-page ChatView, which remains for dedicated chat sessions.

## Consequences

**Positive:**
- Each project is fully self-contained with its own ADRs, agents, skills, and hooks
- New projects are immediately productive — no manual config setup required
- Developers can chat with inference without leaving the project context
- Multi-project management becomes first-class

**Negative:**
- Binary size increases slightly due to embedded templates (~50KB)
- Per-project config means changes to defaults don't propagate to existing projects

**Mitigations:**
- Template embedding is negligible vs existing rust-embed assets (~2MB)
- A future `hex project update-templates` command can refresh defaults selectively

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Project-scoped ADR routes (Rust + frontend) | Pending |
| P2 | Config/skills/agents template copy at init | Pending |
| P3 | Embedded project chat widget | Pending |
| P4 | Build, deploy, verify | Pending |

## References

- ADR-043: Project manifest + auto-registration
- ADR-044: Startup config sync (repo → SpacetimeDB)
- ADR-027: HexFlo native coordination
