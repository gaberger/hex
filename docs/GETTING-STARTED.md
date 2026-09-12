# Getting started

hex is one binary. It needs a Rust toolchain to build and an inference server to
run. Nothing else has to be installed or started.

## Build

```bash
git clone https://github.com/gaberger/hex.git
cd hex
cargo build -p hex-cli --release
export PATH="$PWD/target/release:$PATH"
```

`hex --help` lists the verbs. `hex --version` confirms the build.

## Set up inference

`hex bootstrap` checks prerequisites, starts the local inference server if one is
installed, pulls the models the project configures, and writes `.hex/project.json`.

```bash
hex bootstrap --dry-run    # show what would happen
hex bootstrap              # do it
```

| Flag | Effect |
|---|---|
| `--skip-models` | Do not pull models. They load on first use instead. |
| `--skip-prereq` | Skip OS checks. For CI. |
| `--force` | Restart the inference server even if it is running. |
| `--profile ci` | Use a frontier API instead of a local server. Needs an API key in the environment. |

The models it pulls come from `.hex/project.json`. There is no built-in list. A
project that configures no models gets none, and `hex bootstrap` says so.

A frontier model needs no server. `hex scaffold`, `hex build` and `hex harden`
delegate to a logged-in `claude` CLI, so `claude --version` is the only check.
`hex do` uses the local server first and the frontier path as a fallback.

[Inference](INFERENCE.md) covers providers, tiers, keys and benchmarking.

## Scaffold a project

The floor comes first. It is deterministic and needs no model.

```bash
hex init ./myapp --scaffold --lang rust
cd myapp && cargo test
```

That gives you a manifest, the layer directories, four passing tests, and a
`.hex/ADR-rules.toml` that `hex analyze` will run from now on. Rust, Go and
TypeScript are supported.

Then build your project onto it. This step uses the frontier path.

```bash
hex scaffold "A bookmark service: SQLite store, HTTP API, tag search" \
  --target ./myapp --lang rust --grade A
```

The command exits nonzero unless both gates pass: the test command, and an
architecture grade of A or better.

## Make one gated change

```bash
hex do run "make add() return a + b, not a - b" \
  --file src/lib.rs --evidence "cargo test --test add"
```

hex edits the file, runs the evidence command, and commits only if it exits 0.
Otherwise the edit is reverted. `hex do runs` lists recent runs and their
verdicts.

## Check the shape

```bash
hex analyze .                # grade, boundary violations, rule violations
hex analyze . --exit-code    # nonzero on any violation, for CI
hex graph build .            # build the code graph
hex graph consumers <path>   # who depends on this, before you delete it
```

## Read next

| Document | Covers |
|---|---|
| [README](../README.md) | The problem hex addresses and the evidence |
| [ARCHITECTURE](../ARCHITECTURE.md) | The crates, the loop, the rules |
| [Development workflow](guides/development-workflow.md) | The gate-first pipeline, step by step |
| [Inference](INFERENCE.md) | Providers, tiers, benchmarking |
| [Evidence](EVIDENCE.md) | Every claim, with the command that checks it |
| [Glossary](reference/glossary.md) | The vocabulary, precisely |
