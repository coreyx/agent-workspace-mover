//! The core of agent-workspace-mover.
//!
//! Everything common to all adapters lives here: the path model, plan assembly, the
//! executor, move records and backups, the JSONL scanner, workspace identity, finding
//! what holds a folder, the leftover search, and the `Adapter` and `Platform` traits.
//! It knows nothing about any particular agent, version-control system or editor
//! (requirements.md, 24.0.1); those live in their own adapter crates.

/// The path model: finding and replacing a workspace path in every recorded form.
pub mod paths;

/// A streaming scanner for JSON Lines files that reports top-level fields' spans.
pub mod jsonl;

/// Text read from agent data, wrapped so it can't be printed by accident.
pub mod sensitive;
