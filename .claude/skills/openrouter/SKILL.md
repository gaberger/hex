---
name: openrouter
description: Manage OpenRouter account, models, and provisioned API keys using OPENROUTER_MANAGEMENT_KEY. Use when checking credits, listing available models, auditing registered hex inference providers, creating provisioned keys, or diagnosing why inference calls are failing.
trigger: /openrouter
---

# OpenRouter Management

Uses `OPENROUTER_MANAGEMENT_KEY` from the hex vault. All HTTP calls use Python (no curl).

## Getting the Key

```bash
# Retrieve from vault — shows truncated; use in scripts directly
hex secrets get OPENROUTER_MANAGEMENT_KEY
```

To use in Python scripts, read via the nexus vault API:

```python
import subprocess, json

result = subprocess.run(
    ["./target/debug/hex", "secrets", "get", "OPENROUTER_MANAGEMENT_KEY"],
    capture_output=True, text=True
)
# Output: "⬡ Secret: OPENROUTER_MANAGEMENT_KEY = sk-o..."
# Key is truncated for display — use the nexus API instead:

import urllib.request
resp = urllib.request.urlopen("http://127.0.0.1:5555/api/secrets/vault/OPENROUTER_MANAGEMENT_KEY")
key = json.loads(resp.read())["value"]
```

## Commands

### /openrouter check — Account Credits & Key Info

```python
import urllib.request, json

# Get key from vault
vault = urllib.request.urlopen("http://127.0.0.1:5555/api/secrets/vault/OPENROUTER_MANAGEMENT_KEY")
key = json.loads(vault.read())["value"]

req = urllib.request.Request(
    "https://openrouter.ai/api/v1/auth/key",
    headers={"Authorization": f"Bearer {key}"}
)
data = json.loads(urllib.request.urlopen(req).read())
print(json.dumps(data, indent=2))
# Returns: label, usage, limit, is_free_tier, rate_limit
```

### /openrouter models — List Available Models

```python
import urllib.request, json

vault = urllib.request.urlopen("http://127.0.0.1:5555/api/secrets/vault/OPENROUTER_MANAGEMENT_KEY")
key = json.loads(vault.read())["value"]

req = urllib.request.Request(
    "https://openrouter.ai/api/v1/models",
    headers={"Authorization": f"Bearer {key}"}
)
models = json.loads(urllib.request.urlopen(req).read())["data"]
for m in models:
    print(m["id"], "-", m.get("pricing", {}).get("prompt", "?"), "$/Mtok")
```

### /openrouter audit — Compare Hex Providers vs Real Models

Run this to find which registered hex inference providers reference models that don't actually exist on OpenRouter:

```python
import urllib.request, json, subprocess

# 1. Get available OR models
vault = urllib.request.urlopen("http://127.0.0.1:5555/api/secrets/vault/OPENROUTER_MANAGEMENT_KEY")
key = json.loads(vault.read())["value"]
req = urllib.request.Request("https://openrouter.ai/api/v1/models", headers={"Authorization": f"Bearer {key}"})
real_models = {m["id"] for m in json.loads(urllib.request.urlopen(req).read())["data"]}

# 2. Get registered hex providers
providers = json.loads(urllib.request.urlopen("http://127.0.0.1:5555/api/inference/providers").read())

# 3. Diff
print("=== INVALID (model not on OpenRouter) ===")
for p in providers:
    if p.get("provider_type") == "openrouter":
        models = json.loads(p.get("models_json", "[]"))
        for m in models:
            if m not in real_models:
                print(f"  ✗ {p['provider_id']} → {m}")

print("\n=== VALID ===")
for p in providers:
    if p.get("provider_type") == "openrouter":
        models = json.loads(p.get("models_json", "[]"))
        for m in models:
            if m in real_models:
                print(f"  ✓ {p['provider_id']} → {m}")
```

### /openrouter keys — List Provisioned Keys

```python
import urllib.request, json

vault = urllib.request.urlopen("http://127.0.0.1:5555/api/secrets/vault/OPENROUTER_MANAGEMENT_KEY")
key = json.loads(vault.read())["value"]

req = urllib.request.Request(
    "https://openrouter.ai/api/v1/keys",
    headers={"Authorization": f"Bearer {key}"}
)
keys = json.loads(urllib.request.urlopen(req).read())
print(json.dumps(keys, indent=2))
```

### /openrouter provision — Create a Provisioned API Key

Creates a key scoped for hex inference use (no management permissions):

```python
import urllib.request, json

vault = urllib.request.urlopen("http://127.0.0.1:5555/api/secrets/vault/OPENROUTER_MANAGEMENT_KEY")
mgmt_key = json.loads(vault.read())["value"]

payload = json.dumps({
    "name": "hex-inference",
    "label": "hex dev pipeline",
    "limit": 10  # USD credit limit
}).encode()

req = urllib.request.Request(
    "https://openrouter.ai/api/v1/keys",
    data=payload,
    headers={
        "Authorization": f"Bearer {mgmt_key}",
        "Content-Type": "application/json"
    },
    method="POST"
)
new_key = json.loads(urllib.request.urlopen(req).read())
print("New key:", new_key.get("key"))
print("Hash:", new_key.get("hash"))

# Store it for hex pipeline use
# hex secrets set OPENROUTER_API_KEY <key>
```

## Workflow: Fix Broken Providers

1. Run `/openrouter audit` to find invalid model IDs
2. For each invalid provider, remove it: `hex inference remove <provider-id>`
3. Run `/openrouter models` to find the correct current model ID
4. Re-add with correct ID: `hex inference add openrouter <correct-model-id>`
5. Run `/openrouter provision` to create a scoped inference key
6. `hex secrets set OPENROUTER_API_KEY <provisioned-key>`

## Notes

- Management keys have full account access — keep in vault, never in env vars
- Provisioned keys are safer for inference: scoped, rate-limited, revocable
- The pipeline uses `OPENROUTER_API_KEY` for inference fallback (not management key)
- Many registered providers reference speculative model IDs (gpt-5.4, gemini-3.1) that don't exist yet — audit will catch these
