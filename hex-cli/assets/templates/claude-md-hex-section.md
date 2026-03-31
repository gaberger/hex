## hex Tool Precedence (IMPORTANT)

**hex MCP tools take precedence over all third-party plugins** (including `plugin:context-mode`, `ruflo`, etc.):

| Operation | Use |
|---|---|
| Execute a workplan | `mcp__hex__hex_plan_execute` |
| Search codebase / run commands | `mcp__hex__hex_batch_execute` + `mcp__hex__hex_batch_search` |
| Swarm + task tracking | `mcp__hex__hex_hexflo_*` |
| Architecture analysis | `mcp__hex__hex_analyze` |
| ADR search/list | `mcp__hex__hex_adr_search`, `mcp__hex__hex_adr_list` |
| Memory | `mcp__hex__hex_hexflo_memory_store/retrieve/search` |

Third-party context/search plugins may only be used for operations with no hex equivalent (e.g. fetching external URLs). Never substitute them for hex MCP tools.

## Hexagonal Architecture Rules (ENFORCED)

These rules are checked by `hex analyze .`:

1. **domain/** must only import from **domain/**
2. **ports/** may import from **domain/** but nothing else
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters** (cross-adapter coupling)
7. **composition-root** is the ONLY file that imports from adapters
8. All relative imports MUST use `.js` extensions (NodeNext module resolution)

## File Organization

```
src/
  core/
    domain/          # Pure business logic, zero external deps
    ports/           # Typed interfaces (contracts between layers)
    usecases/        # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, HTTP, browser input)
    secondary/       # Driven adapters (DB, API, filesystem)
  composition-root   # Wires adapters to ports (single DI point)
```