//! A child process launched through a `FixtureHome` must not be able to find the real
//! home folder through any location variable (tasks.md, TASK-1).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use awm_testkit::{FixtureHome, LOCATION_VARS};

/// Runs the probe through `fixture` and returns what it saw: variable name to value,
/// with `None` for an unset variable.
fn probe(fixture: &FixtureHome) -> BTreeMap<String, Option<String>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_awm-testkit-env-probe"));
    let output = fixture
        .apply(&mut command)
        .output()
        .expect("the probe runs");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("the probe prints UTF-8")
        .lines()
        .map(|line| match line.split_once('=') {
            Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
            None => (line.trim_end_matches(" unset").to_owned(), None),
        })
        .collect()
}

#[test]
fn every_location_points_inside_the_fixture_and_away_from_the_real_one() {
    let fixture = FixtureHome::new().unwrap();
    let seen = probe(&fixture);
    for name in LOCATION_VARS {
        let value = seen[*name]
            .as_deref()
            .unwrap_or_else(|| panic!("{name} is set"));
        assert!(
            Path::new(value).starts_with(fixture.root()),
            "{name} is inside the fixture"
        );
        // The fixture usually sits in the system temporary folder, which may itself be
        // under the real home, so check the value isn't the real location itself.
        if let Some(real) = std::env::var_os(name) {
            assert_ne!(
                Path::new(value),
                Path::new(&real),
                "{name} isn't the real location"
            );
        }
    }
}

#[test]
fn variables_that_lead_back_to_the_real_home_are_removed() {
    let fixture = FixtureHome::new().unwrap();
    let seen = probe(&fixture);
    for name in ["HOMEDRIVE", "HOMEPATH", "CLAUDE_CONFIG_DIR"] {
        assert_eq!(seen[name], None, "{name} is removed");
    }
}

#[test]
fn a_relocated_claude_config_dir_is_inside_the_fixture() {
    let fixture = FixtureHome::new()
        .unwrap()
        .with_claude_config_dir()
        .unwrap();
    let seen = probe(&fixture);
    let dir = seen["CLAUDE_CONFIG_DIR"]
        .as_deref()
        .expect("CLAUDE_CONFIG_DIR is set");
    assert!(Path::new(dir).starts_with(fixture.root()));
    assert!(Path::new(dir).is_dir());
}

#[test]
fn every_location_folder_exists() {
    let fixture = FixtureHome::new().unwrap();
    for (name, path) in fixture.locations() {
        assert!(path.is_dir(), "{name}'s folder exists");
    }
}
