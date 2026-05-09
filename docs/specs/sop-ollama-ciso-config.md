# CISO [PERSON_NAME] Configuration for ADR-[PHONE]

## File: `hex-cli/assets/agents/hex/hex/ciso.yml`

Add the following line under the `model:` section:

```yaml
model:
  preferred: claude-opus-4-7
  fallback: deepseek/deepseek-r1
  upgrade_threshold: 0.8
  preferred_provider: ollama  # ADR-[PHONE]: skip OpenRouter content-filter for security work
```

## Rationale

The CISO persona handles security-sensitive language (secret, credential, auth, vulnerability) in 100% of its work. Routing it to [PERSON_NAME] by default eliminates wasted OpenRouter 403 responses and reduces REASON phase latency from ~8s to ~4s.

## Verification

After applying this change:

1. Trigger a CISO SOP ask containing "secret" or "credential"
2. Check trace logs for `SOP REASON: persona prefers ollama, routing directly`
3. Verify no OpenRouter HTTP 403 in logs
4. Confirm adr_draft or escalate_to_operator tool call succeeded

## Rollback

If Ollama tool-calling quality degrades:

```yaml
model:
  preferred_provider: openrouter  # revert to OpenRouter-first
```

Or globally disable fallback:

```bash
export HEX_SOP_OLLAMA_FALLBACK=false
```
