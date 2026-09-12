## hex — how to work in this project

hex is one binary. There is no daemon, no database, and no dashboard. Nothing
needs to be started. Every verb below runs in this process and exits.

`hex --help` lists the verbs. `hex go` suggests the next action.
`hex hey <intent>` routes natural language to a verb.

### Route work through hex where hex has a verb for it

| Don't do this by hand | Use this |
|---|---|
| edit, then hope it works | `hex do run "<task>" --file <f> --evidence "<cmd>"` |
| build a whole component by hand | `hex build '<challenge>' --target <dir> --gate '<cmd>'` |
| eyeball a diff for bugs | `hex harden <path> --gate '<cmd>'` |
| assert "the architecture is fine" | `hex analyze .` |
| delete code you *think* is dead | `hex graph consumers <path>` |
| read a file before editing it blind | `hex graph context <path>` |
| a lesson you will forget | `hex memory store lesson:<topic> "<text>"` |
| claim something about the repo | `hex verify "<claim>"` |

If the verb you want is missing, **build the verb**. That is how the tool grows.

### Autonomous operation (HARD RULES)

1. **Trace consumers before you delete.** `hex graph consumers <path>` over the
   whole workspace, then grep for the two things a graph cannot see: a
   re-export at a package root, and a conditionally-compiled import. Both have
   caused a broken build after a trace came back clean.
2. **Every change that deletes or restructures ends with a build.** A task
   marked done on a broken build is worse than no task.
3. **Never end with a menu of options.** Ship the highest-value item now. Say
   what shipped and what is left. Asking per item stalls the session.
4. **Record work; do not defer it.** Outstanding work goes in a workplan
   (`hex plan draft <prompt>`) or an ADR, now. Never "next session".
5. **No stub tasks.** A task whose body is `echo TODO` is audit theater. Real
   work goes in a workplan; not-yet-actionable work goes in an ADR.
6. **Seek improvements proactively.** Drift or a gap → ADR → workplan.
7. **Prefer `hex hey <intent>`** when the task maps cleanly to natural language.

## Development pipeline (gate-first)

**The executable gate replaces the written spec. A spec that cannot be run is
an ADR.**

1. **Decide** — an ADR in `docs/adrs/`, if this adds a port, an adapter, or a
   dependency.
2. **Gate** — write the command that must exit 0, *before* the code.
3. **Diverge** — `hex build` proposes designs and red-teams each one. The spec
   it synthesizes is a disposable intermediate.
4. **Build to the gate.**
5. **Harden** — `hex harden`: adversarial hunt, refute by default, every fix
   gated.
6. **Ship.**

Three rules govern it:

- **A spec that cannot be run does not exist.** It becomes a gate, or it
  becomes ADR prose. ADR prose is history, and history is allowed to be
  unexecutable, because it never claims to describe the present.
- **The gate is written before the code, and is not derived from it.** A gate
  generated from the implementation tests the implementation against itself.
- **A vacuous gate is a failed gate.** "0 tests passed" is not a pass. Every
  new gate shape needs that guard.

## Hexagonal Architecture Rules (ENFORCED)

`hex analyze .` checks these:

1. **domain/** imports only from **domain/**.
2. **ports/** imports from **domain/** only, for value types.
3. **usecases/** imports from **domain/** and **ports/** only.
4. **adapters/primary/** imports from **ports/** only.
5. **adapters/secondary/** imports from **ports/** only.
6. **Adapters NEVER import other adapters.**
7. The **composition root** is the only file that imports an adapter.
8. Relative imports in TypeScript use `.js` extensions (NodeNext).

Rule 4 has a consequence people miss: if an adapter needs a domain type, the
**port** re-exports it. The adapter imports the port. Otherwise every adapter
grows a second edge into the core.

## File Organization

```
src/
  domain/            # Pure business logic, zero external deps
  ports/             # Typed interfaces — the contracts between layers
  usecases/          # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, HTTP, browser input)
    secondary/       # Driven adapters (DB, API, filesystem)
  composition-root   # Wires adapters to ports — the single DI point
```

## Lessons that a rule cannot catch

`.hex/ADR-rules.toml` enforces the lessons a pattern can find. `hex analyze .`
runs them, and each one cites the incident that produced it. These are the rest
— real failures with no pattern to match. Read them before you trust a green
result.

- **A gate that degrades silently is worse than no gate.** A boundary check
  that falls back to a compile check when its analyzer is unreachable turns "no
  boundary violations" into "it compiles" — a different claim — and still
  prints a result. Fail loudly. *(Not enforceable: no pattern separates a gate
  falling back from an ordinary default for an optional flag. A rule for this
  was written, fired only on correct code, and was removed —
  `.hex/ADR-rules.toml` records why.)*
- **Tests can mirror bugs.** The same model writes the code and the test, so the
  test encodes the same misunderstanding. Use property tests and an independent
  oracle. Worse: a test that redefines its subject asserts against a copy of the
  design, not against shipped code. *(Partly enforced:
  `test-must-not-redefine-its-subject`.)*
- **"It compiles" is not "it works".** Add a check that a user can actually run
  the thing.
- **The write shape and the read shape must agree.** A record persisted with a
  string id and read back into a numeric field fails to deserialize on every
  row. If the read discards errors, the feature reports empty and looks healthy.
  Never discard a deserialize error you did not expect.
- **A cache in front of a file that only a short-lived process reads is not a
  cache.** It is a second source of truth that can disagree with the first.
- **Documentation about a deleted feature never fails.** It sits there being
  confidently wrong. When you delete a feature, grep the docs in the same
  change.
- **Parallelize by file boundary; serialize by file overlap.** Two agents
  editing one file produce conflicting diffs.
- **Sign conventions matter.** For physics and maths, write the coordinate
  system down.
