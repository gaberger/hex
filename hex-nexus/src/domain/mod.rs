//! Domain layer — pure value types with no adapter/transport dependencies.
//!
//! Per the hexagonal rules hex enforces (and now obeys — ADR-2606071340), the
//! `ports/` layer may import only from `domain/`. The transport/agent/inference
//! contract types (`AgentMessage`, `RemoteAgent`, `SshTunnelConfig`,
//! `TransportError`, …) are exactly that contract, so they live here rather than
//! in the `remote/` adapter where the ports were previously reaching up into them.

pub mod transport;
