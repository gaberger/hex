# Workplan: Project-Scoped ADRs, Config Templates, and Embedded Chat

**ADR:** ADR-045
**Commit baseline:** 07bd36a
**Priority:** HIGH — foundational for multi-project management
**HexFlo swarm:** `project-scoped-resources`

## Phase 1: Project-Scoped ADR Routes

**Goal:** ADR browser shows each project's own ADRs.

### Rust Backend

| File | Change |
|------|--------|
| `hex-nexus/src/routes/adrs.rs` | Extract `list_adrs_from_dir(dir)` and `get_adr_from_dir(dir, id)` shared functions. Add `list_project_adrs` and `get_project_adr` handlers that resolve project path from SharedState. |
| `hex-nexus/src/routes/mod.rs` | Register `GET /api/projects/{id}/adrs` and `GET /api/projects/{id}/adrs/{adr_id}` routes. |

### Frontend

| File | Change |
|------|--------|
| `hex-nexus/assets/src/components/views/ADRBrowser.tsx` | When `projectId` is in route, fetch from `/api/projects/{id}/adrs` instead of `/api/adrs`. |

### Acceptance Criteria
- Navigate to project → ADRs tab shows that project's `docs/adrs/` content
- Global ADR browser still works at `#/adrs`

---

## Phase 2: Config/Skills/Agents Copy at Init

**Goal:** New projects start with usable agent defs, skills, settings, and manifest.

### Rust Backend

| File | Change |
|------|--------|
| `hex-nexus/src/templates.rs` (NEW) | `AgentTemplates` and `SkillTemplates` RustEmbed structs pointing to `../agents/` and `../skills/`. |
| `hex-nexus/src/routes/files.rs` | After creating dirs (~line 227), iterate embedded templates and write to project `.claude/agents/`, `.claude/skills/`. Write default `.claude/settings.json` and `.hex/project.yaml` if absent. |
| `hex-nexus/src/lib.rs` | Add `pub mod templates;` |

### Acceptance Criteria
- `POST /api/projects/init` with a new path populates agents, skills, settings, manifest
- Re-running init on existing project does NOT overwrite customized files
- `hex project register /tmp/test` creates all expected files

---

## Phase 3: Project Chat Widget

**Goal:** Embedded chat panel in ProjectDetail view, connected to inference.

### Frontend

| File | Change |
|------|--------|
| `hex-nexus/assets/src/stores/project-chat.ts` (NEW) | Factory `createProjectChat(projectId)` returning signals + WebSocket connection to `/ws/chat?project_id={id}`. |
| `hex-nexus/assets/src/components/chat/ProjectChatWidget.tsx` (NEW) | Compact right-side panel (~350px). Reuses MessageList. Input bar with send. Toggle via floating button. |
| `hex-nexus/assets/src/components/views/ProjectDetail.tsx` | Add `chatOpen` signal, floating toggle button, and `<ProjectChatWidget>` rendering. |

### Acceptance Criteria
- Click chat icon in project detail → panel slides in
- Send message → streaming response from inference server
- Chat is scoped to the project (project_id passed to WebSocket)
- Close button hides panel, main content expands back

---

## Phase 4: Build & Deploy

```bash
cargo build -p hex-nexus --release
hex nexus stop && hex nexus start
```

### Verification Checklist
- [ ] ADRs: project-scoped list + detail works
- [ ] Config: new project gets agents, skills, settings, manifest
- [ ] Config: existing project not overwritten
- [ ] Chat: widget opens, sends messages, receives streaming responses
- [ ] Build: no new compiler errors or warnings
