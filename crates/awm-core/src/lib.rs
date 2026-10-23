//! The core of agent-workspace-mover.
//!
//! Everything common to all adapters lives here: the path model, plan assembly, the
//! executor, move records and backups, the JSONL scanner, workspace identity, finding
//! what holds a folder, the leftover search, and the `Adapter` and `Platform` traits.
//! It knows nothing about any particular agent, version-control system or editor
//! (requirements.md, 24.0.1); those live in their own adapter crates.
