# ADR-2604010003: Bold-colon-outside Status

**Status**: Accepted
**Date:** 2026-04-03
**Drivers:** The colon lands outside the bold delimiters (`**Status**:` rather than `**Status:**`). The strict reader rejects it; the doctor must flag.

## Context

A second variant of the same family of bugs as `unparseable-status-bullet-bold.md`, without the leading bullet. Tested independently because the lenient detector and the auto-fix regex must handle both shapes.
