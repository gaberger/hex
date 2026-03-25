# ADR: Hello World CLI in Rust

## Status
Accepted

## Context
We need a simple CLI application in Rust that prints "Hello, World!" to stdout.

## Decision
Implement a minimal Rust binary using std only, no external dependencies.

## Consequences
- Binary will be fast and have no dependencies
- Simple to build with `cargo build`
