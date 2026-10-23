//! Prints the location variables this process sees, one `NAME=value` or `NAME unset`
//! per line, so tests can check what a child process launched through a
//! `FixtureHome` would find.

use awm_testkit::{LOCATION_VARS, REMOVED_VARS};

/// Prints every location and removed variable as this process sees it.
fn main() {
    for name in LOCATION_VARS.iter().chain(REMOVED_VARS) {
        match std::env::var_os(name) {
            Some(value) => println!("{name}={}", value.to_string_lossy()),
            None => println!("{name} unset"),
        }
    }
}
