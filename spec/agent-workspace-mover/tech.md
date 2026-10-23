---
status: approved
---

# agent-workspace-mover Tech Stack

## Summary

The tool is written in Rust and ships as one self-contained binary per OS and architecture, with no runtime to install. The dependencies are few and permissively licensed:
- `clap` for the command line
- `serde_json` with key order preserved for the JSON files it edits
- Microsoft's `windows` crate and `sysinfo` for processes, locks and file permissions
- ripgrep's `ignore` crate to walk the workspace
- `blake3` for verification hashes
- `rusqlite`, with SQLite built in, and `md-5` for VS Code's state

For transcripts, a small in-house JSON scanner changes one field's value without touching any other byte. git is driven through its own command line. Tests use `cargo test`, with fixture home folders and fault injection. Coverage is measured with `cargo-llvm-cov`, and CI runs on Windows, macOS and Linux. Releases are built by a plain GitHub Actions matrix.

## Decisions

### Language and toolchain
**Choice:** Rust, stable channel. The toolchain is pinned in `rust-toolchain.toml` to the stable release current when implementation starts (1.99.1 on 2026-10-23), and the minimum supported version is declared in `Cargo.toml`. The coverage job alone uses a pinned nightly (see "Testing and coverage").
**Why:**
- **No runtime (REQ-25, 25.0.1).** It produces one native binary per platform. The tool can't assume Node, Python or .NET is installed, and Claude Code's native install doesn't bring any of them. Users download a binary and run it.
- **Direct OS access (REQ-20, REQ-19, REQ-12).** Finding what holds a folder on Windows, setting file permissions, and starting a process that outlives its terminal all need Win32 calls. Rust calls them directly through Microsoft's own bindings, with no add-on layer.
- **Speed (REQ-28).** Planning has to read and hash up to 2 GB of agent data within 15 seconds.
- **Safety where it matters most.** Strong typing and exhaustive `match` make it hard to forget a case in the code that edits irreplaceable data (REQ-27). The owner treats that code as critical, with a 90–100% coverage target.

**Alternatives considered** (laid out for the owner, who chose Rust on 2026-10-23):
- **Go:** a simple single binary, but no native branch coverage (only a beta third-party tool). That leaves the branch-coverage target unmeasurable. Its standard JSON library also loses key order (7.0.4).
- **TypeScript on Node:** closest to the reference script, but needs Node installed, or Node's single-executable mode, which is still in active development. Win32 access would go through an FFI add-on.
- **C# with .NET Native AOT:** strong on Windows and measures branch coverage. Its tooling for macOS and Linux command-line apps is thinner, and AOT and trimming impose restrictions.

### Command line and prompts
**Choice:** `clap` (MIT/Apache-2.0) with its derive API for commands and flags. Prompts are a small in-house module on the standard library: yes/no, the typed opt-in phrase, and a numbered file list for 10.0.3. Terminal detection uses `std::io::IsTerminal`.
**Why:**
- `clap` is the standard Rust command-line parser, actively maintained, and gives consistent help and usage errors (exit code for "usage error", 22.0.2).
- Prompts stay in-house because the typed confirmation is safety-critical (REQ-8). It must read exactly what the user typed, compare it case-sensitively, refuse when input or output isn't a terminal (8.0.4), and be testable by injecting the input. A prompt library adds behaviour to audit (defaults, key handling, fallbacks when there's no terminal) and buys nothing the tool needs.
**Alternatives considered:**
- `dialoguer`: its last release was 0.11 in 2023, so its maintenance is uncertain.
- `inquire`: a capable full-screen interface, but more than a numbered list needs.
- Both add a terminal-rendering dependency to the safety-critical path.

### JSON settings files (`~/.claude.json`, VS Code's JSON)
**Choice:** `serde` and `serde_json` (MIT/Apache-2.0), with the `preserve_order` feature so objects keep their key order, and `arbitrary_precision` so numbers keep their exact text. Files are written back with the indentation found in the original, detected on reading.
**Why:**
- 7.0.4 and 21.0.4 require editing these files as JSON: change only the named keys, keep every other key, value and its order, and write atomically.
- `preserve_order` keeps the files recognisable to their owners and diff-friendly.
- `arbitrary_precision` stops a round trip from rewriting a number's text, for example a large id or a float.
- A file with duplicate keys, or one that fails to parse, is refused rather than repaired (7.0.5).
**Alternatives considered:** text or regex edits, which the notes record failing on `~/.claude.json` in practice, and which can't prove that only the named keys changed. A format-preserving JSON editor that keeps whitespace byte for byte: none is mature in Rust, and the acceptance criteria (7.4, 21.3) compare parsed content, not bytes.

### Transcripts and prompt history (JSONL)
**Choice:** An in-house streaming JSON scanner of a few hundred lines. It reads one line at a time and reports the byte span of a named field's value, only at the top level of the line's object. The opt-in rewrites (REQ-8) splice the new value into exactly that span and copy every other byte unchanged. Reads stream line by line rather than loading whole files.
**Why:**
- 8.0.6 and 8.2 require that an opt-in changes only the named field's value. Conversation text stays as it was, even where it contains the old path or a nested object with a `cwd` key. Re-serialising a line through `serde_json` would change other bytes (escapes, number text, spacing).
- A regex on `"cwd":"…"`, as in the reference script, can't tell the top-level field from the same text inside nested content.
- Line-by-line streaming keeps memory flat across 2 GB of agent data (REQ-28). The same scanner counts recorded paths for plans and leftovers (REQ-3, REQ-16), and classifies `entrypoint` (6.0.9).
- Because this code is critical, it's tested against `serde_json` as the reference: property-based tests check that every span it reports parses to the same value `serde_json` sees, on generated and real-shape lines.
**Alternatives considered:**
- `serde_json`'s `RawValue`: it preserves raw text but doesn't give byte spans of top-level fields within a line.
- Third-party streaming parsers (`struson`, `jiter`): they're capable, but the subset needed is small, the code is critical, and owning it keeps it fully covered and auditable.

### Filesystem operations
**Choice:** The standard library:
- `std::fs::rename` for same-volume moves (1.0.3) and atomic replacement. On Windows it replaces existing files atomically.
- `File::sync_all` before each replace (12.0.3).
- `std::fs::File::lock` to stop two runs working on the same paths at once (13.0.1).
- Volume identity compared before planning, so the plan knows whether a move is a rename or a copy (1.0.4). That's the device id on Unix, and the volume serial via the `windows` crate on Windows.
- `tempfile` (MIT/Apache-2.0) for temporary files created next to their targets, so the final rename never crosses volumes.

**Why:** These are the primitives REQ-12 and REQ-27 need: writes that are never half-done, and no overwriting. Rust's standard library also adds the prefix Windows needs for paths over 260 characters (5.0.6).
**Alternatives considered:** `fs_extra` and similar copy helpers. Cross-volume copying needs per-file verification and journaling (1.0.4, 12.0.2), which a generic helper doesn't give.

### Processes, locks and the session the tool runs in (REQ-20, REQ-2)
**Choice:**
- **Every OS:** `sysinfo` (MIT) to list processes, their names, parents and working folders. On Windows it reads another process's working folder from its process environment block, including 32-bit processes.
- **Windows:** the `windows` crate (MIT/Apache-2.0, maintained by Microsoft):
  - the **Restart Manager** API, to name processes holding files open under the source
  - parent-process walking, to recognise the session the tool was launched from (20.0.3)
- **Linux:** `/proc/<pid>/cwd` and `/proc/<pid>/fd`, read directly.
- **macOS:** `libproc` system calls through the `libc` crate, with `lsof` (always present on macOS) as a fallback.

**Why:**
- On Windows, a held folder makes the move fail (2.0.4, 20.1), so naming the holder is a core feature.
- On macOS and Linux a held folder doesn't block a rename, but REQ-20 still asks for holders to be named, because an editor or a running agent will lose track.
- `sysinfo` saves writing the undocumented process-block reading by hand, which a mature, widely used crate already does on Windows.

**Alternatives considered:**
- Enumerating every open handle on the system (`NtQuerySystemInformation`): finer-grained, but undocumented, slow, and fragile across Windows versions.
- Shelling out to Sysinternals `handle.exe`: not installed by default, and has its own licence terms.

### Finishing a move later (REQ-19)
**Choice:**
- **Windows:** the waiting process is started by a one-off, per-user **Task Scheduler** task, created through the `windows` crate's Task Scheduler interface. It waits on the blocking processes' handles. A directly detached child process is the fallback, where the parent's job allows it to break away.
- **macOS and Linux:** the waiting process is double-forked into a new session (`setsid`), so it survives the terminal and the agent session closing.

**Why:**
- 19.0.2 requires the waiting process to survive the terminal or agent session that started it. On Windows, a console host or an agent may place its children in a job that kills them on close. A child can leave such a job only if the job allows it, and that isn't known in advance.
- A task started by Task Scheduler runs outside the caller's job entirely, and creating one for the current user needs no administrator rights (25.0.1).

**Alternatives considered:**
- A Windows service: needs administrator rights.
- Registry "run" keys: only run at the next sign-in.
- Leaving the waiting to the agent's own session: the session is what's blocking the move.

### Walking and searching the workspace (REQ-10, REQ-11)
**Choice:**
- `ignore` (MIT/Unlicense, by the author of ripgrep): a parallel directory walk that respects `.gitignore`, `.git/info/exclude` and global ignore files, even when git isn't installed (9.0.6).
- `memchr` and `aho-corasick` (MIT/Unlicense): fast multi-pattern literal search for every recorded form of the source path (5.0.1).
- `regex` (MIT/Apache-2.0): checks each candidate match to confirm the path ends where a whole segment ends (5.0.3).
- Text encoding: detected in-house (UTF-8 with or without a byte-order mark, and UTF-16 with a byte-order mark). Other encodings are reported, not edited (10.0.7).

**Why:**
- REQ-28 needs a 100,000-file workspace planned within 15 seconds. These are the crates ripgrep uses for exactly this kind of search.
- `ignore` gives correct ignore-file behaviour without calling git once per file.

**Alternatives considered:**
- `git ls-files` and `git check-ignore`: need git installed and don't cover folders that aren't repositories.
- `walkdir` with hand-written ignore rules: reimplements gitignore's semantics, and gets them wrong.

### Version control (REQ-9, REQ-17)
**Choice:** Drive the `git` command line (version 2.29 or later, which added `git worktree repair`) for repairs and identity queries: `rev-list --max-parents=0`, `remote -v`, `worktree list --porcelain`. When git is missing or too old, the tool reads `.git` files directly for detection and identity only, never to change anything, and reports the repair command instead (9.4).
**Why:**
- `git worktree repair` is the authoritative implementation of the link format, and reimplementing it would drift as git changes.
- Repairs run as a separate process, so a failure is contained and reported (9.0.6).
- `.git/config` and the worktree link files are simple and stable to read for detection.

**Alternatives considered:**
- `gix` (gitoxide) and `git2` (libgit2): neither supports `worktree repair`. Both add a large dependency to reproduce what the command line does authoritatively.

### VS Code state (REQ-21)
**Choice:**
- **The recent list:** `rusqlite` (MIT) with its `bundled` feature, which compiles SQLite (public domain) into the binary, to read and write the `history.recentlyOpenedPathsList` row of `globalStorage/state.vscdb`. That row's value is JSON, edited with `serde_json` like any other settings file. The edit runs in a single transaction that touches only that row (21.0.4).
- **Workspace storage ids:** `md-5` (MIT/Apache-2.0, RustCrypto) computes the id VS Code gives each folder (21.0.2), from the folder's path and its creation time, or its inode on Linux.
- **Creation times:** on a move across volumes, the creation time is carried across with the `windows` crate on Windows and with `setattrlist` through `libc` on macOS (21.0.7).

**Why:**
- VS Code keeps its recent list in SQLite, not in `storage.json`, and keys each folder's state by an id computed from its path (requirements.md, section 3).
- Bundling SQLite avoids depending on whatever version, if any, the OS provides, and keeps the single-binary promise (REQ-25).
- MD5 is used only because it's VS Code's own formula, not for any security purpose.

**Alternatives considered:**
- Shelling out to the `sqlite3` command line: not installed by default on Windows.
- Rewriting the database file's bytes: unsafe for a page-structured format.
- Leaving VS Code as report-only: considered and set aside by the owner on 2026-10-23.

### Hashing and verification
**Choice:** `blake3` (CC0/Apache-2.0) for content hashes: verifying cross-volume copies (1.0.4), verifying backups (12.0.1), and fingerprinting state for undo and finish-later checks (14.0.3, 19.0.3).
**Why:** It's fast enough to hash 2 GB inside REQ-28's limits, and it has a reference-quality Rust implementation. The hashes are for detecting accidental change, not for security, so a fast cryptographic hash is ample.
**Alternatives considered:** SHA-256 (`sha2`), roughly several times slower in software. It's the better choice only if hashes ever need to match outside tools. They don't.

### Backups, records and file permissions (REQ-12, REQ-26)
**Choice:**
- **Records:** the tool keeps its move records and backups under the user's local application-data folder: `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, and `$XDG_STATE_HOME` (or `~/.local/state`) on Linux. Paths are worked out in-house from those variables. Records are JSON, written atomically, one file per step.
- **Permissions:** on Windows, the `windows` crate sets a protected access-control list granting access only to the current user's account. On macOS and Linux, modes `0600` and `0700` are set through `std::os::unix::fs::PermissionsExt`.

**Why:**
- 12.0.4 and 12.3 require backups to be local and readable only by the user, and on Windows that means setting the access-control list explicitly.
- JSON records can be inspected by hand in an emergency, which matters most when the tool itself has failed (REQ-13).

**Alternatives considered:**
- An embedded database such as SQLite: records stop being readable by hand, and it adds a native dependency.
- The `directories` crate: it's a small, stable job done in a few lines, and owning it avoids a dependency.

### Machine-readable output (REQ-22)
**Choice:** `serde` types for the plan, report, scan and status, with `schemars` (MIT) generating the published JSON Schema from those same types. The schema is checked into the repository, and CI fails if it changes without a schema version change (22.0.4).
**Why:** Generating the schema from the types that produce the output keeps the two from drifting (22.1).
**Alternatives considered:** a hand-written schema, which drifts.

### Errors and logging
**Choice:** `thiserror` (MIT/Apache-2.0) for typed errors in library code, mapped to exit codes in one place. A small in-house logger writes to a local log file under the tool's state folder. There's no logging framework.
**Why:**
- Every exit code in 22.0.2 has to be produced deliberately, which typed errors make checkable.
- A redaction rule (REQ-26) is easier to enforce at a single logging point than through a general framework's formatting layers.

**Alternatives considered:**
- `anyhow`: loses the error types needed for the exit-code mapping.
- `tracing`: more than a command-line tool needs, and its many formatting layers make redaction harder to audit.

### Agent skill (REQ-23)
**Choice:** `SKILL.md` lives in the repository as Markdown and is built into the binary with `include_str!`, so `skill install` (23.0.4) needs no network and always matches the binary's version.
**Why:** 26.0.1 rules out downloading anything, and a skill that matches its binary can't describe flags the binary doesn't have.

### Testing and coverage
**Choice:**
- **Unit and integration tests:** `cargo test`, with:
  - `tempfile` and `assert_fs` for fixture folders
  - `assert_cmd` for driving the binary end to end
  - `insta` (Apache-2.0) for snapshot tests of plans and reports
  - `proptest` (MIT/Apache-2.0) for property-based tests of path matching (REQ-5) and of the JSONL scanner against `serde_json`
- **Fixture homes:** every test runs against generated fake home folders, through location overrides that point the tool at a fixture instead of the real home folder, `~/.claude` (`CLAUDE_CONFIG_DIR`), `%APPDATA%` and similar. Overrides change where the tool looks, never whether an opt-in applies (8.0.5, 8.5). Tests never touch real agent data.
- **Fault injection:** the `fail` crate (Apache-2.0), compiled in only for tests, to stop a move at every step boundary (12.1, 12.2, 13.1).
- **Coverage:** `cargo-llvm-cov`, measuring line and region coverage on stable in every CI run. A separate scheduled job on a pinned nightly adds `--branch`, since branch coverage still needs nightly. Per-module thresholds are enforced in CI: 90–100% for the modules that write agent data, backups, records and undo, and the standard targets elsewhere (testing standards).
- **Network-free check (26.1):** the full end-to-end suite runs on the Linux CI runner inside a network namespace with no interfaces (`unshare -n`). `cargo-deny` also rejects any network client crate in the dependency tree.
- **Agent-skill evaluation (23.2, 23.3):** a scripted evaluation that runs real Claude Code headless against a fixture workspace. It needs an API key, so it runs locally or on demand, not in public CI.

**Why:** these are the tools the full-tier testing standards call for, applied to what REQ-12, REQ-13 and REQ-26 make critical.
**Alternatives considered:** `cargo-tarpaulin`, which is weaker on Windows than `llvm-cov`.

### Continuous integration and releases
**Choice:**
- **CI:** GitHub Actions on Windows, macOS and Linux runners, running tests, clippy with warnings as errors, `rustfmt`, coverage, `cargo-deny` (licences, advisories, banned crates), and the schema check.
- **Builds:** release binaries for `x86_64` and `aarch64` on Windows (MSVC), macOS and Linux (statically linked against `musl`).
- **Releases:** a plain workflow matrix builds them, then publishes archives with SHA-256 checksums and build-provenance attestations to GitHub Releases, plus the crate to crates.io (the name `agent-workspace-mover` was free on 2026-10-23).
- **Install routes:** the documented default routes are ones that don't mark the download as coming from the internet, so neither SmartScreen nor Gatekeeper checks it. All are set up for the first public release:
  - `cargo install agent-workspace-mover`
  - a Homebrew tap
  - a Scoop bucket and a winget manifest
  - `curl` (macOS, Linux) and PowerShell (Windows) install scripts that download from GitHub Releases and verify the checksum

  Downloading an archive from GitHub Releases in a browser is documented as a secondary route, with what to expect on each OS.
- **Windows code signing:** apply to the [SignPath Foundation](https://signpath.org/), which signs open-source projects free of charge, at the first release. Once accepted, add its signing step to the release workflow. Its conditions are already met by the plan: an OSI-approved licence (MIT), automated builds straight from the repository, and a released version. The first release may therefore ship unsigned. Signing keys stay on SignPath's hardware, and each signing request is approved by hand.
- **macOS code signing:** deferred. Developer ID signing and notarisation need the Apple Developer Program (US$99 a year). They're only needed for binaries downloaded in a browser, which the default routes avoid. Revisit if there's evidence that users download that way. Apple Silicon's requirement for at least an ad-hoc signature is met automatically by the Rust toolchain's linker.

**Why:**
- REQ-25 needs the whole suite passing on all three OSes (25.1).
- Static Linux builds run on any distribution.
- Checksums and attestations let users verify a binary that's going to handle their data.
- Routes that don't mark downloads avoid the warnings without paid signing. The free Windows signing covers the one case routes can't, Smart App Control (see "Constraints & risks").

**Alternatives considered:**
- `cargo-dist`, which automates releases and installers. Upstream development stalled in 2025 and moved to a community fork, so its maintenance is uncertain. A plain matrix is a few dozen lines of workflow that we control. Worth revisiting at the first release.
- [Azure Artifact Signing](https://azure.microsoft.com/pricing/details/artifact-signing/) for Windows, at about US$9.99 a month. Individuals qualify only in the US and Canada, and new files still need time to build SmartScreen reputation. It's the fallback if SignPath doesn't accept the project.
- An interpreted language, so that the signed `node` or `python` runs unsigned scripts. It brings back the runtime dependency REQ-25 rules out. The native add-on needed for Win32 calls would still be unsigned, and Smart App Control blocks those. Node's single-executable mode produces an executable that needs signing again.

## Constraints & risks

- **Branch coverage needs nightly.** The branch figures come from a pinned nightly job, not the stable build. If nightly's coverage output changes, the branch thresholds may need re-baselining. Line and region coverage on stable stay the gating metric for every pull request.
- **Reading another process's working folder on Windows is undocumented.** It relies on the process environment block, which `sysinfo` reads. It fails for processes the user can't open, such as elevated terminals when the tool isn't elevated, and protected processes. REQ-20 handles this by saying it can't tell, rather than claiming the folder is free (20.0.4). On Windows, the rename itself remains the final test, since it either succeeds or fails cleanly.
- **The Restart Manager works on files, not folders.** It names processes holding files open, but not processes whose working folder is inside the source. Those come from the working-folder check above. Registering 100,000 files with the Restart Manager may be slow, so the design should limit it to likely candidates and measure against REQ-28.
- **Task Scheduler can be disabled by policy** on managed machines. Finishing later on Windows then falls back to a detached process, which may not survive if the caller's job forbids breaking away. The tool must detect this and say that finishing later isn't available, rather than start a waiter that will be killed.
- **On Linux, systemd may kill a user's processes at logout** (`KillUserProcesses`). A waiting move then ends at logout, and the design should detect and report this.
- **git 2.29 or later is needed for repairs.** Older gits still allow the move, with repairs reported as commands to run (9.4).
- **macOS and Linux holder detection is less proven.** It's built on documented interfaces, but verified only against fixtures until it's used on real machines (25.0.2, 25.0.3).
- **Unsigned binaries, until Windows signing is in place.**
  - SmartScreen and Gatekeeper mostly check only files a browser downloaded, and the default install routes avoid those (see "Continuous integration and releases").
  - **Windows 11's Smart App Control is the exception.** Where it's on, it blocks unsigned or little-known executables however they're launched, terminal included. Only signing fixes that.
  - Until the SignPath signing step is in place (the first release, at least), users with Smart App Control on can't run the tool. The README must say so.
  - If SignPath declines or withdraws the project, the fallback is Azure Artifact Signing, whose eligibility depends on where the owner is.
  - macOS binaries stay unsigned beyond the ad-hoc signature, so a browser download will be quarantined. The README documents how to clear it, or to use a default route instead.
- **Rust is a steeper language for contributors than Go or TypeScript.** The adapter interfaces (REQ-24) should be small and well documented, with one complete adapter as the example, so adding an agent doesn't require knowing the whole codebase.
- **Supply chain.** Every dependency is MIT, Apache-2.0, Unlicense or CC0, or public domain (SQLite), enforced by `cargo-deny`. `Cargo.lock` is committed, and no dependency's build script may download anything. `rusqlite`'s bundled SQLite is compiled from C at build time, so the release builds need a C compiler, which every CI runner image provides.
- **VS Code's formats are internal.** The workspace storage id formula and the recent list's row are VS Code implementation details, and could change in any release. The adapter checks its formula against existing folders before re-keying any (21.0.5), and treats a recent-list value it can't parse as unrecognised (REQ-24).
