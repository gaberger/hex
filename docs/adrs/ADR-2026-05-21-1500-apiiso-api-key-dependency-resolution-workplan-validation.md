# ADR-2026-05-21-1500 — APIISO API Key Dependency Resolution Workplan Validation

Status: **Proposed**
Date: 2026-06-04

## Context
Three workplan_emit failures occurred CISO role due to missing ANTHROPIC_API_KEY/OPENROUTER_API_KEY. These API keys are required for workplan validation but have been absent since at least ADR-2026-05-08-2500.

## Decision
We propose to:n1. [�� Accept operator and dependency with secure vault mechanism2. 🔴 Remove workplan_emit dependency on external APIs
3. ⚛️ Pause workplan_emit until keys are available

Chosen option: [ ] to decide via escalation #1780601293996]

## Consequences
- Option 1: Maintains current workflow but introduces secure key management
- Option 2: Break current integration pipeline but improves external dependency
- Option 3: Delays work workplan operations until keys are available