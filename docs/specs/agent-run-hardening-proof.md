# Agent Run Hardening — End-to-End Proof

Date: 2026-05-14
Repo: /home/[PERSON_NAME]/hex-intf
Driver: ADR-[PHONE] (dashboard refactor on Hermes model) and the operator directive "harden the agent run loop".

This spec captures the moment `hex agent run` became operationally usable for code-patch dispatch via natural-language operator intent.

## What was broken

Before commit `ed5dfd16` (approximate — verify with git log), `/api/inference/complete` accepted a `tools` field in its request struct but never forwarded it to the upstream provider. The simple_agent loop therefore looped on prose-only responses, hit max_iterations or no_tool_use, and never produced a tool dispatch. End result: zero on-disk artifacts from `hex agent run` invocations.

## What was fixed

Three surgical changes:
1. hex-nexus/src/routes/chat.rs — new `call_inference_endpoint_with_tools` helper translating Anthropic schema to OpenAI function-calling shape and parsing tool_calls back from the response.
2. hex-nexus/src/routes/inference.rs — tools fast-path branch at the top of inference_complete, routing directly through OpenRouter when body.tools is non-empty.
3. hex-nexus/src/orchestration/simple_agent.rs — extract_tool_uses now recognizes top-level tool_calls field; system prompt rewritten as dual-protocol (native preferred, fenced JSON fallback).

## Empirical proof

Run `hex agent run "Use code_patch to ..."` and observe a file land on disk plus an autonomous commit on main with subject `chore(...): auto — action#N → <basename>`. This document itself is such an artifact.