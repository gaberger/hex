# Inference

hex names no model in its source. Every model is read from configuration, and
every provider lives in one crate, `hex-infer`. This page covers the
configuration.

## Providers

`hex config inference` manages the registry at `~/.hex/inference-servers.json`.

```bash
hex config inference list                       # what is registered
hex config inference add ollama                 # local server, default URL
hex config inference add ollama http://gpu-box:11434 --model qwen2.5-coder:14b
hex config inference add openrouter --model meta-llama/llama-4-maverick
hex config inference test <id>                  # send one prompt, confirm a reply
hex config inference discover                   # scan for local and remote servers
hex config inference remove <id>
```

Provider types: `ollama`, `vllm`, `openai-compat`, `openrouter`, `groq`,
`cerebras`, `sambanova`, `together`, `gemini`. Every cloud provider speaks the
OpenAI chat protocol through one adapter.

### API keys

Keys are environment variables. hex reads them at dispatch and writes them to no
file.

| Provider | Variable |
|---|---|
| OpenRouter | `OPENROUTER_API_KEY` |
| Groq | `GROQ_API_KEY` |
| Gemini | `GEMINI_API_KEY` |
| Anthropic | `ANTHROPIC_API_KEY` |

The local server's address is `OLLAMA_HOST`. It accepts `host:port` or a full
URL. hex normalises both.

### The frontier path

`hex scaffold`, `hex build` and `hex harden` delegate to a logged-in `claude`
CLI. That path needs no API key and has no memory ceiling. `hex do` reaches it as
a fallback when the local candidates fail.

## Tiers

A task's tier selects a model. The mapping lives in `.hex/project.json`.

```json
{
  "inference": {
    "tier_models": {
      "t1":   "qwen3:4b",
      "t2":   "gemma4-12b",
      "t2.5": "devstral-small-2:24b"
    },
    "react_models": ["devstral-small-2:24b", "claude-code"]
  }
}
```

| Tier | Work |
|---|---|
| `t1` | Scaffolding, transforms, classification. The cheapest model that works. |
| `t2` | Ordinary code generation. |
| `t2.5` | Cross-file reasoning and design. |
| `t3` | Frontier work. Handled by the `claude` path, not by this table. |

`react_models` is the ordered candidate list for `hex do`. The loop runs each in
turn and commits the first whose edit passes the gate. The gate picks the
winner, not a classifier, so a wrong order costs latency and nothing else.

A project that configures no model for a tier gets an error naming the missing
key. There is no default model anywhere in the source.

## Benchmarking

Leaderboard rank does not predict performance in the agent loop. Measure.

```bash
hex bench agentic                          # the corpus, through the real loop
hex bench agentic --model qwen2.5-coder:14b --arms fast
hex config inference bench <id>            # code-gen, reasoning, identity prompts
```

`hex bench agentic` runs each fixture in an isolated worktree and reports edit
rate and evidence pass rate per model. The corpus is in `docs/benchmarks/`.

## The local ceiling

The same task, three languages, per-model pass rate on a 16 GB GPU:

| Model | Rust | TS | Go |
|---|---|---|---|
| devstral-small-2:24b | 5/5 | 3/3 | 2/3 |
| gemma3:12b | 4/5 | 2/3 | 1/3 |
| qwen2.5-coder:14b | 0/5 | 2/3 | 0/3 |
| gpt-oss:20b | 0/5 | 1/3 | 0/3 |

TypeScript is forgiving. Rust and Go are strict, and weaker models fail both.
The top-of-the-leaderboard local model scored last. The ceiling depends on your
language, and `hex bench agentic` tells you where it is on your hardware.

## Hardware

Memory is the constraint before compute. A 24B model at Q4 needs about 15 GB
resident. A 14B model needs about 9 GB. The resource governor in `hex-exec`
checks available memory before each candidate and routes to the frontier path
when the local model will not fit.

`hex config inference gpu-check` confirms the local server is using the GPU.
