//! Fake processes for tests that need something holding a folder: a process whose
//! working folder is inside it, or one with a file in it open.
//!
//! A fake process is the calling test binary itself, run again with one ignored test
//! selected: the `awm_fake_process` test that [`fake_process_entry!`] defines. That
//! way no extra binary has to be built for each crate that needs one. Each test crate
//! that uses [`FakeProcess`] invokes the macro once.

use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Set in a fake process's environment, so `awm_fake_process` knows to act as one.
pub const FAKE_ENV: &str = "AWM_FAKE_PROCESS";

/// The file a fake process holds open, when set.
pub const HOLD_ENV: &str = "AWM_FAKE_HOLD";

/// The line a fake process prints once it's holding what it was asked to.
const READY: &str = "awm-fake-process: ready";

/// A running fake process, killed when dropped.
#[derive(Debug)]
pub struct FakeProcess {
    child: Child,
}

impl FakeProcess {
    /// Starts a fake process whose working folder is `cwd`, holding `hold` open if
    /// given, and waits until it says it's ready.
    pub fn start(cwd: &Path, hold: Option<&Path>) -> io::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "awm_fake_process",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .current_dir(cwd)
            .env(FAKE_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(file) = hold {
            command.env(HOLD_ENV, file);
        }
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().expect("stdout is piped");
        for line in BufReader::new(stdout).lines() {
            if line?.contains(READY) {
                return Ok(Self { child });
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(io::Error::other(
            "the fake process exited before it was ready; is fake_process_entry!() invoked in this test crate?",
        ))
    }

    /// The fake process's id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for FakeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The body of the `awm_fake_process` test: when run as a fake process, holds the
/// requested file open, says it's ready, and waits to be killed. When run any other
/// way, does nothing.
pub fn fake_process_main() {
    if std::env::var_os(FAKE_ENV).is_none() {
        return;
    }
    let _held = std::env::var_os(HOLD_ENV)
        .map(|file| std::fs::File::open(file).expect("the file to hold opens"));
    println!("{READY}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    // The parent kills this process; the limit only stops an orphan lingering.
    std::thread::sleep(std::time::Duration::from_secs(120));
}

/// Defines the ignored `awm_fake_process` test that [`FakeProcess`] runs. Invoke it once
/// in each test crate that starts fake processes.
#[macro_export]
macro_rules! fake_process_entry {
    () => {
        #[test]
        #[ignore = "a fake process for other tests; see awm_testkit::FakeProcess"]
        fn awm_fake_process() {
            $crate::fake_process::fake_process_main();
        }
    };
}
