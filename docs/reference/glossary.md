# Glossary

The words hex uses, and what they mean precisely. Where a term has a looser everyday sense, the "not" column says which one to
avoid, so docs and CLI output stay consistent.

## The method

| Term | Means | Not |
|---|---|---|
| **gate** | A shell command that must exit 0. The only authority on whether work counts. Written before the code and not derived from it. | "test suite" alone. A gate may be a build, a lint, a benchmark, or a script |
| **evidence** | The output of running a gate. `hex do --evidence "<cmd>"` names the gate for one task. | "proof", "verification" generically |
| **vacuous gate** | A gate that exits 0 having exercised nothing, such as a test filter matching zero tests. Detected and rejected. | "empty test" |
| **floor** | The deterministic skeleton: a manifest, the layer directories, passing tests, and a gate command, all from templates compiled into the binary. No inference involved. | "boilerplate", "template" alone. The floor is gated and reproducible |
| **floor gate** | The skeleton's own gate, run *before* any model call. If it fails, nothing measured afterwards means anything. | "smoke test" |
| **architecture grade** | 0–100 score from the boundary analyzer, rendered A+ through F. It is a gate, not a report. `--grade A` fails the build below it. | "score", "health" alone |
| **rule** | A pattern check in `.hex/ADR-rules.toml`, run by `hex analyze`, citing the incident that produced it. Ships with every scaffolded project. | "lint rule". A rule here names a dependency or a defect class, not a style |
| **harden** | An adversarial pass: hunt by failure-class lens, verify each finding skeptically (default-refute), fix the confirmed ones under the gate. | "review", "audit" alone |

## Hexagonal architecture

| Term | Means | Not |
|---|---|---|
| **port** | A typed interface contract between layers. Zero implementation. | "API", "service", "interface" alone |
| **adapter** | An implementation of a port for one technology (HTTP, filesystem, database). | "plugin", "driver", "provider" |
| **primary adapter** | Driving adapter. Accepts external input (CLI, HTTP, UI). Imports `ports/` only. | "controller", "inbound" alone |
| **secondary adapter** | Driven adapter. Calls external systems (database, API, filesystem). Imports `ports/` only. | "repository" generically, "outbound" alone |
| **domain** | Pure business logic. Imports only other `domain/` modules. | "model", "entity layer" |
| **usecase** | Application logic composing ports. Imports `domain/` and `ports/` only. | "service", "handler" |
| **composition root** | The single file that wires adapters to ports. The **only** file that may import an adapter. | "config", "bootstrap", "DI container" |
| **port re-export** | A port exposing a domain type so adapters can use it without importing `domain/` directly. This is how rule 4 is satisfied. | "type alias". The point is the dependency edge, not the naming |

## Inference

| Term | Means | Not |
|---|---|---|
| **tier** | A routing class: T1 scaffold/transform, T2 codegen, T2.5 reasoning, T3 frontier. Resolved from `.hex/project.json`. | "model size" alone |
| **best-of-N** | Run an ordered list of candidate models and commit the first whose gate passes. The gate picks the winner, not a classifier. | "N-shot sampling" |
| **frontier path** | Delegation to a logged-in `claude` CLI, with no API key and no VRAM ceiling. Used when local models cannot finish, and by the build harness throughout. | "fallback" alone. It is also the primary path for whole-system builds |
| **the inference boundary** | `hex-infer`. The only crate where a provider or model name may appear. Everything else reads a tier. | "the model layer" |

## Governance

| Term | Means | Not |
|---|---|---|
| **ADR** | Architecture Decision Record, in `docs/adrs/`. It is append-only. A changed decision gets a new ADR that supersedes the old one. | "design doc", "RFC" alone |
| **epoch** | A design era grouping ADRs. Marks when the shape of the system changed, so an old decision can be read in context. | "release", "version" |
| **founding goal** | A goal in `founding-goals.md` that hex exists to serve. It is the one artifact agents may not author or amend. Retiring one needs a human commit and a Retirement-ADR. | "requirement", "principle" alone |
| **Retirement-ADR** | The ADR that explains why a founding goal no longer serves the project. Required before that goal may be removed. | "deprecation notice" |

## Reading the CLI

| Term | Means |
|---|---|
| **`hex init --scaffold`** | Write the floor only. Deterministic, no model call. |
| **`hex scaffold`** | The floor, then your project built onto it, gated on the build **and** the grade. |
| **`hex build`** | A subsystem from one description, built to a gate you supply. |
| **`hex do`** | One bounded change to one file, committed only if your gate exits 0. |
| **`hex harden`** | The adversarial pass over existing code. |
| **`hex analyze`** | The grader: boundary violations, architecture grade, rule compliance. |
| **`hex graph consumers`** | Who depends on this path. Run it before deleting anything. |
