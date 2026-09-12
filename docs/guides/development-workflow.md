# Development workflow

The pipeline has six steps. A gate is written before the code and is not derived
from it. That order is the whole method.

```
Decide → Gate → Diverge → Build → Harden → Ship
```

## 1. Decide

A change that adds a port, an adapter, or a dependency gets an ADR in
`docs/adrs/`. The ADR records the decision and the reasoning. It is history and
is allowed to be prose, because it never claims to describe the present.

```bash
hex adr schema            # the template and the next number
hex adr list              # every ADR and its status
hex adr search <term>
hex adr accept <id>       # Proposed → Accepted
```

ADRs are append-only. A changed decision gets a new ADR that supersedes the old
one.

## 2. Gate

Write the command that must exit 0. It is the only authority on whether the work
is finished, so it comes before the code, and it is not generated from the code.

```bash
cargo test --test bookmarks       # a test that does not exist yet
```

A gate that passes having run nothing is rejected. `hex do` and `hex scaffold`
both check for a vacuous pass.

## 3. Diverge

`hex scaffold` and `hex build` propose several designs and red-team each one
before building. The spec they synthesize is a disposable intermediate. It is
not kept and not maintained.

```bash
hex scaffold "<what to build>" --target <dir> --lang rust --designs 2
hex build "<subsystem>" --target <dir> --gate "<cmd>" --designs 3
```

## 4. Build

The frontier path writes the code. The gate decides whether it counts. For a
bounded change to one file:

```bash
hex do run "<task>" --file <path> --evidence "<cmd>"
```

hex edits, runs the command, and commits only on exit 0. A failed edit is
reverted, so each attempt starts from the original file.

For a scaffolded project there is a second gate. `--grade A` fails the build if
the architecture grade falls below it.

## 5. Harden

The adversarial pass hunts the result for bugs its own tests missed. Each
finding is verified skeptically before any edit, and each fix is gated.

```bash
hex harden <path> --gate "<cmd>"
hex build "<subsystem>" --target <dir> --gate "<cmd>" --harden   # both in one
```

On this project's rate limiter it found three real bugs behind fourteen passing
tests. One was a `u128` overflow accepted on a confidently wrong comment.

## 6. Ship

```bash
hex analyze . --exit-code     # grade and violations, nonzero on any
hex graph consumers <path>    # before deleting anything
cargo test --workspace
```

Then commit. The gate has already decided the work is done. The commit records
it.

## What is not in the pipeline

There is no spec step. A spec that cannot be run does not exist. It becomes a
gate, or it becomes ADR prose.

There is no swarm, no task board, and no daemon. Every verb runs in-process and
exits.
