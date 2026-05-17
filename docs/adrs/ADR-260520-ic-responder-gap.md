# ADR-260520-ic-responder-gap

## Status: Proposed

## Context

Today, in `hex-nexus/src/orchestration/org_responder.rs`, lines 80-85 only poll inboxes for the roles of cto/cpo/coo/ciso/chief-visionary. This limitation affects the other 26 personas listed in `hex-cli/assets/agents/hex/hex/` (dashboard-ux-architect, hex-coder, hex-fixer, hex-tester, etc.), who are registered in the persona pool and receive direct messages (DMs) that never get replies. Evidence of this issue includes:

1. A message from CTO to dashboard-ux-architect on 2026-05-15 regarding a dashboard UX issue that has been unread for 2 days.
2. Another message from CEO to dashboard-ux-architect on 2026-05-17 asking about a Kanban filter (message 126995) that remains stuck.

## Problem

The current IC responder system is limited to only five executive roles, while the rest of the organization's personas are not being addressed adequately. This gap leads to silent IC asks that remain unanswered, hindering effective communication and operational flow within the organization.

## Considered Options

1. **Widen the Responder Allowlist**: Modify the existing responder system to include all roles in the organizational chart.
2. **Add a Sister IcResponder Daemon**: Introduce a separate daemon designed specifically for handling the response of non-executive personas.
3. **Add a Per-Persona Supervisor**: Implement a per-persona supervisor that reuses the standard SOP path, tailoring the system prompt to each role using the YAML persona definitions.

## Decision

We will implement option 2: **Add a Sister IcResponder Daemon**.

### Rationale

- **Separation of Concerns**: By adding a separate daemon, we can maintain clarity and separation between the handling of executive roles and other non-executive roles.
- **Scalability**: This approach is scalable and allows for future enhancements specific to each role without impacting the existing system.
- **Data-Driven Design**: We will refactor the existing executive prompt-builder (lines 108-111 and 266-269 in `org_responder.rs`) to be data-driven, using YAML files to define prompts for all roles. This will make the system more flexible and easier to maintain.

### Implementation

1. **Create a New Daemon**: Develop a new daemon that will handle responses for non-executive personas.
   - Location: `hex-nexus/src/orchestration/ic_responder_daemon.rs`
   - Functionality: Poll inboxes for all roles listed in the persona pool and respond based on their YAML definitions.

2. **Refactor Prompt Builder**:
   - Modify the existing prompt-builder to use a data-driven approach.
   - Update `org_responder.rs` to load prompts from YAML files, ensuring that each role receives a system prompt flavored to its specific needs.

3. **Integrate and Test**: Integrate the new daemon with the existing system and conduct thorough testing to ensure that all roles are being responded to appropriately.

### Timeline

- **Week 1**: Design and development of the sister IcResponder daemon.
- **Week 2**: Refactoring the prompt builder and integrating the new daemon.
- **Week 3**: Testing and validation.
- **Week 4**: Deployment and monitoring for any issues.

## Conclusion

By implementing a sister IcResponder daemon, we address the gap in communication between non-executive personas and the organization. This solution ensures that every silent IC ask is responded to, improving overall operational efficiency and effectiveness.

## References

- `hex-nexus/src/orchestration/org_responder.rs`
- `hex-cli/assets/agents/hex/hex/` (list of personas)
- YAML persona definitions in `hex-cli/assets/agents/hex/hex/`

---

Drafted by: cto
Date: 2026-05-20
