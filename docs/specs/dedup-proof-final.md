# Dedup Proof — Final

Date: 2026-05-14
Repo: /home/[PERSON_NAME]/hex-intf
Driver: ADR-[PHONE] (Hermes-aligned dashboard refactor).
Verb: hex agent run.

This file proves the auto-stop-on-duplicate-success guard in simple_agent collapses N spurious code_patch retries into a single autonomous commit. The model often forgets to call finish; the runtime now catches the dup signature and terminates the loop cleanly.

After your single code_patch call returns ok:true, STOP. Do not retry.
