# Tiered Inference Routing

Source ADRs: 2604120202 + 2604131630. Skill: `/hex-inference`.

Tasks classify into tiers mapped to progressively more capable models.

| Tier | Default Model | Use Case |
|------|--------------|----------|
| T1 | `qwen3:4b` | Scaffold, transform, script — boilerplate |
| T2 | `qwen2.5-coder:32b` | Standard codegen — adapters, tests |
| T2.5 | `devstral-small-2:24b` | Complex reasoning — cross-adapter wiring, architecture |
| T3 | Claude (frontier) | Frontier tasks — bypasses scaffolded dispatch |

## Tier selection

- `strategy_hint` on WorkplanTask controls tier directly:
  - `scaffold` / `transform` / `script` → T1
  - `codegen` → T2
  - `inference` → T2.5
- When absent, heuristics use layer depth + dependency count.

## Scaffolded dispatch (T1 / T2 / T2.5)

Best-of-N: each candidate must pass a compile gate (`cargo check` / `tsc --noEmit`) before acceptance.

T3 bypasses scaffolding — frontier models produce single-shot output.

## Configuration

Override defaults per-tier in `.hex/project.json`:

```json
{
  "inference": {
    "tier_models": {
      "t1": "qwen3:4b",
      "t2": "qwen2.5-coder:32b",
      "t2_5": "devstral-small-2:24b"
    }
  }
}
```

## Monitoring

```bash
hex inference escalation-report   # how often tasks escalate up the tiers
```

High escalation rates ⇒ tier thresholds need tuning.
