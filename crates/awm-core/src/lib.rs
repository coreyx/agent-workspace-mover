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

/// Where the home folder, agent data, editor data and the tool's state live.
pub mod locations;

/// OS-specific code, the only place unsafe code is allowed.
pub mod platform;

/// Writing files so they're never left half-written.
pub mod atomic;

/// UTC timestamps for records, the log and move ids.
pub mod timestamp;

/// The tool's state folder and the machine-wide lock.
pub mod state;

/// Move records and content backups.
pub mod record;

/// The tool's log, which only accepts values that are safe to keep.
pub mod log;

#[cfg(test)]
awm_testkit::fake_process_entry!();

/// The plan and the report, and their schemas.
pub mod plan;

/// The adapter interface, and assembling a plan from adapters' contributions.
pub mod adapter;

/// The executor: the only code that changes the machine.
pub mod executor;

/// The core's own checks before a move, and the folder move they lead to.
pub mod checks;
