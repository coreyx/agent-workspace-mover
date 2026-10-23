---
status: approved
---

# Implementation Tasks: agent-workspace-mover, version 1

This document defines the implementation plan as a numbered checklist. Each task is linked to one or more acceptance criteria from [`requirements.md`](./requirements.md). The architecture, decisions and interfaces the tasks build are in [`design.md`](./design.md), and the stack is in [`tech.md`](./tech.md).

Every task includes its own unit and integration tests, written with the tools `tech.md` names and run against fixture homes from `awm-testkit`, never real agent data. A task is done when its acceptance criteria pass in CI on Windows, macOS and Linux, and its code meets the coverage targets in the testing standards. Modules that write agent data, backups, records or undo state are critical, with a 90–100% target: the executor, the record, backups, the JSONL scanner, the JSON and SQLite editing, and the Claude Code and VS Code adapters' actions. TASK-34 makes those thresholds gates in CI.

The design lists ten facts to verify ("Facts to verify during implementation", F-1 to F-10). Milestone 1 checks them before any code relies on them. A fact that turns out wrong is raised with the owner before the dependent task starts, because it may change a requirement.

## Task List

### Milestone 1: Facts verified and the project set up

Once this is done, the workspace builds and tests in CI on all three OSes, and every assumption the design rests on has been confirmed or raised with the owner.

- [ ] **TASK-1: Set up the Cargo workspace and CI**
  - **Associated Acceptance Criteria**: none (project setup)
  - **Concrete Steps**:
    1. Create the workspace with the crates in design.md's "Crates" section: `awm`, `awm-core`, `awm-agent-claude`, `awm-vcs-git`, `awm-editor-vscode`, `awm-detectors` and `awm-testkit`. Each starts as an empty, documented skeleton.
    2. Pin the toolchain in `rust-toolchain.toml` (1.99.1), declare the minimum supported Rust version in `Cargo.toml`, and commit `Cargo.lock`.
    3. Add a GitHub Actions workflow for Windows, macOS and Linux that runs `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`.
    4. Add `cargo-deny`, configured to allow only MIT, Apache-2.0, Unlicense, CC0 and public-domain licences, and to ban network client crates.
    5. Add `cargo-llvm-cov` line and region coverage on stable for every run, and a scheduled pinned-nightly job with `--branch`, reporting only for now (TASK-34 adds the gates).
    6. Give `awm-testkit` a fixture-home builder that sets `HOME`/`USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, `XDG_*` and `CLAUDE_CONFIG_DIR` for a child process, plus a test proving a child process launched through it can't see the real home folder.

- [ ] **TASK-2: Verify Claude Code facts F-1 to F-5**
  - **Associated Acceptance Criteria**: none (discovery; results update design.md's facts table and the Claude Code adapter's declared verification)
  - **Concrete Steps**:
    1. F-1, with the owner at a terminal: start a real interactive session in a throwaway folder and run a shell command, so the session records the POSIX form of `cwd`. Then move the folder, merge its project folder, and have the owner confirm the `claude --resume` picker lists the session from the new folder. Clean up, as in the 2026-10-22 experiment.
    2. F-2: create throwaway folders whose encoded names are about 190, 200, 210 and 250 characters, run a short `claude -p` session in each, and record the project-folder name Claude Code creates. Find any shortening rule. Clean up.
    3. F-3: with `CLAUDE_CONFIG_DIR` set to a throwaway folder, run a short session, and record where Claude Code reads and writes `.claude.json`, `projects/`, `history.jsonl` and `sessions/`.
    4. F-4: create a git worktree under `.claude/worktrees/` in a throwaway repo, run a session in it, and record its project folder's name and the `cwd` values it writes.
    5. F-5: record the process names, executable paths and argument patterns of the native CLI, an npm-installed CLI if available, the VS Code extension and the desktop app, from a process listing. Never read `sessions/*.json` contents.
    6. Write the results into a short findings note in the spec folder, linked from design.md's facts table. Raise any fact that contradicts the design with the owner before TASK-12 starts.

- [ ] **TASK-3: Verify VS Code facts F-6 and F-7**
  - **Associated Acceptance Criteria**: none (discovery; results update design.md's facts table and the VS Code adapter's declared verification)
  - **Concrete Steps**:
    1. F-6: in a throwaway script, compute the workspace storage id for every folder in `workspaceStorage/*/workspace.json` on this machine, using VS Code's formula (MD5 of `fsPath` followed by its creation time). Record how many match their folder name, and the exact `fsPath` form that matches, including the drive-letter case. Read only.
    2. F-7: read the `history.recentlyOpenedPathsList` row of `globalStorage/state.vscdb` from a copy of the file, and record the shape of its JSON (key names and URI forms only, not values).
    3. F-7: in a copy of one workspace's `state.vscdb`, list which `ItemTable` keys hold file URIs under its folder, and whether they hold them as plain JSON strings or as JSON nested inside strings.
    4. Write the findings note. If open editors' URIs can't be rewritten as structured JSON, raise 21.1's "open editors" example with the owner before TASK-19.

- [ ] **TASK-4: Verify platform facts F-8 to F-10**
  - **Associated Acceptance Criteria**: none (discovery; results set the time budgets and fallbacks in TASK-8 and TASK-28)
  - **Concrete Steps**:
    1. F-8: write a spike in `awm-testkit`'s examples that registers 1,000, 10,000 and 100,000 files with the Restart Manager in batches, and measure the time on the CI Windows runner and this machine. Choose the batch size and the planning budget.
    2. F-9: write a spike that creates and runs a one-off Task Scheduler task as a standard user, from Windows Terminal and from inside Claude Code's shell tool. Check that the task's process survives closing the terminal or ending the Claude Code session. Also check whether the calling process is in a job that allows breaking away.
    3. F-10: on the macOS CI runner, write a spike that reads another process's working folder and open files through `libproc`, without extra rights, and that reads a volume's case sensitivity. Note anything that needs the `lsof` fallback.
    4. Write the findings note, and raise any fact that forces a design change with the owner.

### Milestone 2: A plain folder moves safely

Once this is done, `agent-workspace-mover move` checks, plans, confirms and moves a folder with no agent data on any supported OS. It records every step, and `plan` shows exactly what would happen.

- [ ] **TASK-5: Implement the path model**
  - **Associated Acceptance Criteria**: `5.1`, `5.2`, `5.3`
  - **Concrete Steps**:
    1. Implement `PathMapping`, `PathForm` and `CaseRule` in `awm-core::paths`, with `find`, `rewrite` and `covers` as design.md's "Interfaces" section specifies.
    2. Support every form: native, forward slashes, JSON-escaped, POSIX drive (`/c/…`), `file://` URIs, and VS Code's percent-encoded `file:///c%3A/…`.
    3. Implement whole-segment boundaries, so `repo-old` and `repo.bak` never match `repo`. Preserve the form and drive-letter case of each replaced match.
    4. Add `proptest` properties: rewriting a match and mapping it back gives the original, and no match ever ends inside a segment. Add unit tests for every form on Windows-style and POSIX-style paths.

- [ ] **TASK-6: Implement the JSONL field scanner**
  - **Associated Acceptance Criteria**: none (foundation for 6.8, 8.2 and 8.6; this task's own tests check it against `serde_json`)
  - **Concrete Steps**:
    1. Implement a streaming, line-at-a-time scanner in `awm-core` that reports the byte span and decoded value of a named field at the top level of each line's object, and ignores the same name in nested objects and strings.
    2. Report unparseable lines with their line numbers, and tell an unterminated final line apart from a malformed one.
    3. Implement splicing: replace the bytes of a reported span with a new JSON string value, copying every other byte unchanged.
    4. Add `proptest` tests comparing every reported span and value with `serde_json`'s parse of the same line, on generated lines and on real-shape fixture lines (nested `cwd` keys, escaped paths, Unicode, very long lines).
    5. Wrap every value read in `AgentBytes` (design.md, "Key design decisions", 11). Add a compile-fail test showing it can't be formatted with `Display`.

- [ ] **TASK-7: Implement locations, the state folder, the lock, records and backups**
  - **Associated Acceptance Criteria**: `12.3`
  - **Concrete Steps**:
    1. Implement `Locations::from_env` for every OS: home, the Claude Code data folder (honouring `CLAUDE_CONFIG_DIR`, as F-3 found), VS Code's user folder, and the tool's state folder.
    2. Create the state folder layout from design.md's "On disk" section. Set user-only permissions on creation: a protected access-control list on Windows through the `windows` crate, and `0700`/`0600` elsewhere.
    3. Implement the machine-wide lock with `File::lock`, and the move-record writer: `move.json`, `plan.json` and `steps/NNNN.json`, each written atomically.
    4. Implement backups: copy, verify by `blake3`, record in the step's file, store under `backups/`.
    5. Implement the redacted logger, which accepts only paths, field names, counts, ids and the tool's own messages.
    6. Test the permissions on each OS (12.3), atomic replacement under a simulated crash, and lock exclusivity between two processes.

- [ ] **TASK-8: Implement the platform layer: processes, holders, volumes**
  - **Associated Acceptance Criteria**: `20.1`, `20.3`
  - **Concrete Steps**:
    1. Implement `Platform::processes` with `sysinfo`: process id, start time, name, executable, arguments, parent, and working folder (`None` when unreadable).
    2. Implement `open_file_holders`: the Restart Manager in batches with the budget chosen in TASK-4 (Windows), `/proc/<pid>/fd` (Linux), and `libproc` with an `lsof` fallback (macOS). Return `complete: false` when the budget runs out or a process can't be inspected.
    3. Implement `volume_id`, `case_rule`, `creation_time` and `set_creation_time` for each OS.
    4. Implement the ancestor walk that marks the tool's own session chain (used by TASK-13).
    5. Test with `awm-testkit` fake processes: a shell whose working folder is under a fixture folder, and a process holding a file open, both named with their process ids (20.1). Add a test that no process file's contents reach output or the log (20.3).

- [ ] **TASK-9: Implement the plan model, adapter trait and output**
  - **Associated Acceptance Criteria**: `22.1`, `22.3`
  - **Concrete Steps**:
    1. Implement the `Adapter` trait, `Contribution`, `PlannedAction`, the closed `Action` enum, `Destination`, `JsonEdit`, `Precondition`, `Blocker` (with stable codes), `Notice`, `LeftUnchanged` and `OptInOffer`, as design.md's "Interfaces" section specifies.
    2. Implement plan assembly: snapshot, then inspect every registered adapter, then merge contributions into one `Plan` with counts per adapter and phase.
    3. Implement the human-readable renderer, with "Will do", "Left unchanged" (with the opt-in or flag for each), "Detected, not handled" and "Blockers" sections.
    4. Implement `--json` output from the same types, with `schemars` generating `schema/awm.plan.v1.json` and the report, scan and status schemas. Add the CI check that fails when a schema changes without a version change.
    5. Implement the exit-code mapping in one place in `awm`, following design.md's exit code table.
    6. Test that `plan --json` validates against its schema (22.1) and that standard output holds exactly one JSON document with human messages on standard error (22.3).

- [ ] **TASK-10: Implement the executor and moving the workspace folder**
  - **Associated Acceptance Criteria**: `1.3`, `1.4`, `5.4`, `5.5`, `12.1`
  - **Concrete Steps**:
    1. Implement the executor's phases in design.md's "Order of steps": recheck, back up, then each phase in order. Write each step's record before and after it runs.
    2. Implement precondition checking at the recheck and before each action, including compare-and-swap for file edits.
    3. Implement `MoveTree` for the same volume: a single rename, with sharing-violation and access-denied errors mapped to a full holder diagnosis (design.md, "Key design decisions", 9).
    4. Implement `MoveTree` across volumes: copy to `.awm-partial-<move id>` preserving times, verify every file by size and `blake3`, rename into place, then delete the source. If verification fails, remove the partial copy and exit 5 (1.4).
    5. Implement each action type's probe (not started, done or partial), starting with `MoveTree`.
    6. Add `fail` points at every step boundary. Test that a backup exists, with the right hash, before each change (12.1); that a same-volume move performs one rename (1.3); a cross-volume move with injected verification failure (1.4); a source reached through a junction or symlink (5.4); and a Windows path over 260 characters (5.5).

- [ ] **TASK-11: Implement the checks and the `move` and `plan` commands**
  - **Associated Acceptance Criteria**: `1.2`, `1.6`, `2.1`, `2.2`, `2.3`, `2.6`, `3.4`, `3.5`, `3.6`, `4.2`
  - **Concrete Steps**:
    1. Implement the core checks: target exists, source missing, one under the other, the refused locations (drive root, home, folders containing home, agent data folders), permissions, free space for a cross-volume copy, and holders. Collect every blocker before stopping (2.0.9).
    2. Implement `move`, `plan` and `move --dry-run` in `awm` with `clap`: confirm with the go-ahead prompt, honour `--yes`, and stop with exit 3 when there's no terminal and no `--yes`.
    3. Implement the `Terminal` type (opened only when standard input and output are both terminals) and the yes/no prompt, reading from an injectable input for tests.
    4. Implement the final report with its "done", "left unchanged" and "couldn't do" sections, always present (1.6), in both renderers.
    5. Test each criterion end to end with `assert_cmd` against fixture folders. That includes a hashed listing before and after `plan` (4.2), and three different blockers reported in one run (2.6).

### Milestone 3: Claude Code's data moves with the workspace

Once this is done, a moved workspace keeps its Claude Code sessions, memory and settings. The record stays unchanged unless the user types an opt-in's phrase, and the tool proves the result by verifying it and searching for leftovers.

- [ ] **TASK-12: Claude Code adapter: data discovery, versions and project-folder matching**
  - **Associated Acceptance Criteria**: `6.5`, `6.6`, `6.8`, `24.2`, `24.3`
  - **Concrete Steps**:
    1. Implement `AdapterInfo` with the verified set (2.1.309 on Windows 11, plus whatever TASK-2 confirms) and the recognised range (`>=2.1.0, <2.2.0`). Read the installed version with `claude --version`, with a timeout, and the versions recorded in transcripts.
    2. Implement project-folder matching as design.md's "Key design decisions", 7, specifies: candidates by encoded name, real paths from top-level `cwd` values via the scanner, exact-name-only for folders with no transcripts, and ambiguity reporting. Use F-4's worktree findings.
    3. Implement the blocker for encoded names past the length found in F-2 (6.0.8), or Claude Code's shortening rule if F-2 found one.
    4. Implement per-folder counts of sessions the picker lists and sessions reachable only by id, from `entrypoint` (6.8).
    5. Implement the shape checks and the version gate with `--allow-unverified claude-code`, including the unterminated-final-line notice (design.md, "Key design decisions", 8).
    6. Test the sibling `repo-old` fixture (6.5), a `CLAUDE_CONFIG_DIR` fixture (6.6), an out-of-range version (24.2), and a malformed line reported by file and line number without its contents (24.3).

- [ ] **TASK-13: Claude Code adapter: running processes and the tool's own session**
  - **Associated Acceptance Criteria**: `2.4`, `4.3`, `20.2`
  - **Concrete Steps**:
    1. Implement `Adapter::running` from F-5's findings (process names, executables, arguments), plus the process ids named by `sessions/<pid>.json` file names, checked against live processes. File names only, never contents.
    2. Add the `agent-running` blocker for any plan that changes Claude Code data while any Claude Code process runs.
    3. Mark a running process found in the tool's own ancestor chain as "this session", and add the `--when-free` offer to the blocker's remedy. The flag itself comes in TASK-28.
    4. Test with fake `claude` processes in `awm-testkit`: a blocked move with exit 4, a dry run completing with exit 3 (2.4, 4.3), and a run launched from a fake session whose working folder is the source (20.2).

- [ ] **TASK-14: Claude Code adapter: carrying session folders and memory**
  - **Associated Acceptance Criteria**: `2.5`, `6.1`, `6.2`, `6.3`, `6.4`, `6.7`
  - **Concrete Steps**:
    1. Implement `MergeDir` in the executor: a rename when the destination is absent, otherwise a recursive merge. Identical files are de-duplicated and listed, folders merge, and different files with the same name are collisions (design.md, "Key design decisions", 15). Record pre-existing destination entries in the step's effects.
    2. Implement `MergeDir`'s probe and collision checks while planning (2.5).
    3. Contribute a `MergeDir` per matched project folder, including worktree folders, to the folder name for the corresponding target path.
    4. Test merging into a target holding only an empty `memory/` (6.4), identical hashes of subagent transcripts, tool results and memory after a move (6.3), and transcripts unchanged by a default move (6.7).
    5. Add an end-to-end test that runs `claude -c` in the target folder against a fixture made by a real short session (6.1). It runs only when an opt-in environment flag and a `claude` binary are present, so public CI skips it. Record the manual picker check (6.2) in the walkthrough's manual steps, using F-1's procedure.

- [ ] **TASK-15: Claude Code adapter: carrying settings**
  - **Associated Acceptance Criteria**: `1.1`, `3.3`, `7.1`, `7.2`, `7.3`, `7.4`, `7.5`
  - **Concrete Steps**:
    1. Implement `EditJsonFile` with `serde_json` (`preserve_order`, `arbitrary_precision`): indentation detected and kept, duplicate keys refused, compare-and-swap on write, and the inverse edits recorded in effects.
    2. Contribute `RenameKey` edits for every `projects` key under the source, unless `--fresh-settings` is given. Contribute `ReplaceString` edits for `githubRepoPaths`, always. Add a collision blocker for an existing target key.
    3. Add the plan's settings notice, listing the settings that move and the `--fresh-settings` flag (3.3).
    4. Test 7.1–7.5, including a parse-level diff showing changes only in the planned keys (7.4), and a file modified between planning and writing (7.5).
    5. Add the full end-to-end fixture for 1.1: a repository with three sessions, memories and settings, all arriving at the target, with nothing left but the listed leftovers.

- [ ] **TASK-16: Opt-ins with typed confirmation**
  - **Associated Acceptance Criteria**: `3.2`, `8.1`, `8.2`, `8.3`, `8.4`, `8.5`, `8.6`
  - **Concrete Steps**:
    1. Implement `TypedConfirmation`, with its private constructor reachable only through `prompt::confirm_phrase(&Terminal, phrase)`, and make `Executor::apply` skip any opt-in without a matching confirmation (design.md, "Key design decisions", 10).
    2. Implement `SpliceJsonlField` with the scanner: record each spliced line number, offset, original value and written value, and back up each file first.
    3. Contribute the `rewrite-sessions` offer (top-level `cwd` in every transcript) and the `rewrite-history` offer (`project` in `history.jsonl`), with counts per file. Contribute "left unchanged" items with those counts when not requested (3.2).
    4. Show the exact changes and counts before the phrase prompt. Compare case-sensitively, and skip on any other input. With no terminal, or with `--json`, report the command to run instead.
    5. Test 8.1–8.6. Include a test that enumerates every environment variable the binary reads and every `clap` argument, and asserts that none produces a confirmation (8.5), and a test that `live_cwd` and `git_state.root` are unchanged after `rewrite sessions` (8.6).

- [ ] **TASK-17: Verification and the leftover search**
  - **Associated Acceptance Criteria**: `16.1`, `16.2`, `16.3`, `27.2`
  - **Concrete Steps**:
    1. Implement `Adapter::verify` for Claude Code: the expected transcript counts at the target, and settings under the target key.
    2. Implement the core leftover search over each adapter's roots, using `PathMapping::find`, reporting counts per file and, for JSON and JSONL, the names of the fields where matches sit.
    3. Classify matches against the adapters' expected leftovers, and raise unexpected ones as warnings with exit 1 (16.2).
    4. Make a failed verification after all steps complete report "needs attention" with exit 6, not success (27.2).
    5. Test 16.1–16.3, including a sentinel string in an unknown file appearing only as a count (16.3).

### Milestone 4: Git, VS Code, workspace paths and the detectors

Once this is done, every v1 adapter takes part in a move: git's links are repaired, VS Code's recent list and saved state follow the folder, paths inside the workspace are offered for update, and the report-only detectors name what will break.

- [ ] **TASK-18: Git adapter**
  - **Associated Acceptance Criteria**: `1.5`, `3.1`, `9.1`, `9.2`, `9.3`, `9.4`
  - **Concrete Steps**:
    1. Detect main repositories, linked worktrees and submodules from `.git` folders and files, including nested repositories under the source (1.5).
    2. Implement `RunTool` in the executor, with its declared inverse and its probe. Contribute `git -C <repo> worktree repair <mapped worktree paths>` for worktrees inside and outside the source, and from the main repository when the source is itself a worktree.
    3. Read `.git/config`, `.git` files, `.git/modules/*/config` and `.git/worktrees/*/gitdir` for absolute paths under the source, and report those `worktree repair` doesn't cover (9.3).
    4. When git is missing or older than 2.29, report the repair command as a "couldn't do" item (9.4).
    5. Implement `Adapter::verify` with `git worktree list --porcelain`.
    6. Test 9.1–9.4 against fixture repositories built with the real `git` in CI. Test a plan showing counts for sessions, settings and worktrees together (3.1), and a folder holding two repositories with sessions (1.5).

- [ ] **TASK-19: VS Code adapter**
  - **Associated Acceptance Criteria**: `21.1`, `21.2`, `21.3`, `21.4`, `21.5`, `21.6`
  - **Concrete Steps**:
    1. Implement `EditSqliteJsonRow` with `rusqlite` (bundled): a single transaction changing only the named row, compare-and-swap on the row's value, inverse edits recorded.
    2. Contribute edits to `history.recentlyOpenedPathsList` (`RewritePathStrings`) and to `storage.json` window state and backup workspaces, in the URI forms F-7 found.
    3. Implement the storage id formula with `md-5`, using F-6's `fsPath` form, and the self-check that refuses all re-keying when any existing folder's id doesn't reproduce (21.4).
    4. Contribute a `MergeDir` per workspace storage folder to a deferred destination. Resolve it after the folder move through `Adapter::resolve`, using the inode on Linux. Carry the creation time across volumes (21.6). Add a collision blocker for an existing target id (21.5).
    5. Contribute `RewritePathStrings` edits for the per-workspace `state.vscdb` rows F-7 found, recursing into JSON held in strings. Add the `vscode-running` blocker (21.2), and notices for Insiders and VSCodium.
    6. Test 21.2–21.6 against fixtures built from F-7's shapes. Add the manual check for 21.1 (recent list and restored state in real VS Code) to the walkthrough's manual steps.

- [ ] **TASK-20: Absolute paths inside the workspace**
  - **Associated Acceptance Criteria**: `10.1`, `10.2`, `10.3`, `10.4`
  - **Concrete Steps**:
    1. Walk the workspace with `ignore` (parallel), skipping `.git`, binary files (a NUL byte in the first 8 KB) and files over 10 MB (reported as skipped). Include ignored files only with `--include-ignored`.
    2. Search with `PathMapping::find`. Detect UTF-8 (with or without a byte-order mark) and UTF-16 (with a byte-order mark), and report other encodings as not edited.
    3. Implement record-like detection (changelogs, logs, release notes, transcripts, lock files), unselected by default, and the masked preview function (design.md, "Key design decisions", 11).
    4. Implement the numbered selection prompt, and `--update-paths <file…>` for runs without a terminal.
    5. Implement `ReplaceInText`, with recorded offsets for undo, a backup, and the file's line endings and byte-order mark preserved.
    6. Test 10.1–10.4.

- [ ] **TASK-21: Report-only detectors**
  - **Associated Acceptance Criteria**: `11.1`, `11.2`, `11.3`, `11.4`
  - **Concrete Steps**:
    1. Implement detectors in `awm-detectors` as adapters that contribute notices only: `pyvenv.cfg` within three levels of the workspace, Pipenv's and Poetry's environment folders for this path, Compose files without `name:` (and `COMPOSE_PROJECT_NAME` in `.env`), direnv allow entries and mise trust entries naming paths under the source, and JetBrains' recent-projects lists and per-project system folders.
    2. Give each notice what the user can do (for example, recreate the virtual environment, add `name:`, run `direnv allow`), and whether the detector is verified on this OS.
    3. Report "couldn't check" when a detector can't read what it looks for (11.0.3).
    4. Test 11.1–11.3 with fixtures, and 11.4 with a hashed listing of every detector's files before and after a move.

### Milestone 5: Interrupted moves recover, and moves can be undone

Once this is done, stage 1 is complete (move, dry run and undo). Any interruption can be resumed or rolled back, and any completed move can be undone, carrying data written after it back with the rest.

- [ ] **TASK-22: `resume` and `rollback`, with fault injection across every action**
  - **Associated Acceptance Criteria**: `12.2`, `13.1`, `13.2`
  - **Concrete Steps**:
    1. Treat a move found as `Applying` when the lock is taken as interrupted. Refuse new mutating runs on the same paths, naming `resume` and `rollback` (13.2).
    2. Implement `resume`: probe the interrupted step, complete it or skip it if it's done, re-check the remaining preconditions, and continue.
    3. Implement `rollback`: probe, then reverse the completed steps in reverse order using each action's structural inverse (design.md, "Key design decisions", 5), offering backups where a check fails.
    4. Make sure every action type (`MoveTree`, `MergeDir`, `EditJsonFile`, `EditSqliteJsonRow`, `SpliceJsonlField`, `ReplaceInText`, `RunTool`) has a probe and an inverse with tests.
    5. Build the fault-injection suite: a fixture move using every adapter, killed at every `fail` point. For each point, check that `rollback` restores the hashed listing from before the move, and that `resume` matches an uninterrupted run (12.2, 13.1).

- [ ] **TASK-23: `undo`**
  - **Associated Acceptance Criteria**: `14.1`, `14.2`, `14.3`, `14.4`
  - **Concrete Steps**:
    1. Build an undo plan from a completed move's record: each step's inverse, in reverse order, as a new move of kind `undo` that references the original.
    2. Run it through the same checks, plan, go-ahead and `--dry-run` as `move` (14.0.2). Make files whose structural checks fail into blockers, with their backups offered (14.0.3).
    3. List before the go-ahead anything that can no longer be reversed after `backups prune` (14.0.4). Refuse when a later move involves the same paths (14.4).
    4. Mark the original move `Undone` when the undo completes.
    5. Test 14.1–14.4. That includes a new session started at the target after the move, carried back (14.2), and the transcripts' original content restored after `rewrite sessions` (14.3).

- [ ] **TASK-24: `history` and backups**
  - **Associated Acceptance Criteria**: `12.4`
  - **Concrete Steps**:
    1. Implement `history` and `backups list`: date, kind, source, target, state and backup size for each move.
    2. Implement `backups prune <move id>` and `backups prune --older-than <age>`, with a go-ahead prompt that lists what can't be undone afterwards.
    3. Make sure nothing removes backups automatically, anywhere.
    4. Test that backups from a fixture move dated 90 days ago remain until pruned (12.4).

### Milestone 6: Orphaned data can be found and rescued

Once this is done, stage 2 is complete. The tool recognises workspaces by identity, lists agent data whose folder is gone, and reattaches it to where the workspace lives now.

- [ ] **TASK-25: Workspace identity**
  - **Associated Acceptance Criteria**: `17.1`, `17.2`, `17.3`
  - **Concrete Steps**:
    1. Implement `Adapter::identity` for git: root commits (`git rev-list --max-parents=0 HEAD`) and remote URLs, normalised (scheme, credentials and `.git` suffix removed). Without git, use remotes from `.git/config` only.
    2. Implement evidence gathering from Claude Code's orphaned data: `githubRepoPaths` keys, the last segment of the recorded path, and branch names recorded in transcripts.
    3. Implement the confidence ratings (high, medium, low) with the evidence shown for each, low for folders with no version control.
    4. Strip credentials from every remote URL before it's stored, shown or logged.
    5. Test 17.1–17.3.

- [ ] **TASK-26: `scan`**
  - **Associated Acceptance Criteria**: `18.1`, `18.2`, `18.3`
  - **Concrete Steps**:
    1. Implement `Adapter::orphans` for Claude Code (project folders and `projects` keys whose real path no longer exists) and for VS Code (workspace storage whose folder no longer exists).
    2. Implement `scan`: size, session count and last activity for each orphan, grouped with temporary-folder leftovers separately, plus likely current locations from parent folders, `--search` folders and the home folder to a limited depth, within a time budget.
    3. Print a `rescue` command for each orphan with a likely match. `scan` never deletes anything.
    4. Test 18.1–18.3, including a hashed listing of all agent data before and after (18.3).

- [ ] **TASK-27: `rescue`**
  - **Associated Acceptance Criteria**: `15.1`, `15.2`, `15.3`, `15.4`
  - **Concrete Steps**:
    1. Implement `rescue <old> <current>`: a plan without `MoveTree`, built from every adapter's inspection with the old path as the source, including git repairs for worktrees that record the old path.
    2. Implement `rescue <current>`: candidates from `scan` that match the workspace's identity, shown with evidence and confidence for the user to choose. With no terminal, list them and change nothing (15.3).
    3. Route rescue through the same checks, go-ahead, backups, opt-ins, record and `undo` as `move` (15.0.4).
    4. Test 15.1–15.4 against a fixture repository moved by hand.

### Milestone 7: Blocked moves finish themselves, and agents know how to use the tool

Once this is done, stage 3 is complete. A move held up by an editor, a terminal or the agent's own session finishes once they exit, and an agent asked to move a workspace uses the tool safely.

- [ ] **TASK-28: Finishing a move later**
  - **Associated Acceptance Criteria**: `19.1`, `19.2`, `19.3`, `19.4`, `19.5`
  - **Concrete Steps**:
    1. Implement `move --when-free` and `--wait-limit`: plan and get the go-ahead in the foreground, write `pending/<move id>.json` with the blocking processes (process id plus start time), and start the waiter. Report "finishing later isn't available here" when `spawn_waiter` can't start one.
    2. Implement `Platform::spawn_waiter`: a Task Scheduler task on Windows, with the detached fallback from F-9's findings, and a double fork with `setsid` on macOS and Linux. Add a notice when logind kills user processes at logout.
    3. Implement the internal `__wait` command (design.md, "Key design decisions", 16): wait for the processes to exit, re-check for new agent processes, debounce, take the lock, re-plan without a terminal, compare kinds of action and blockers, then apply or stop. Record the outcome.
    4. Implement `status` and `cancel`, and enforce one waiting move per source.
    5. Test 19.1–19.5 with fake holder processes, including closing the starting terminal (19.2) on each OS's CI runner, and an opt-in requested but never applied (19.5).

- [ ] **TASK-29: The agent skill and `skill install`**
  - **Associated Acceptance Criteria**: `23.1`
  - **Concrete Steps**:
    1. Write `SKILL.md` in the Agent Skills format, covering every instruction in 23.0.2 and 23.0.3, using the stable blocker codes and the exit codes from design.md.
    2. Embed it in the binary with `include_str!`.
    3. Implement `skill install`: show the destination under the Claude Code data folder, ask for the go-ahead, then write it atomically.
    4. Validate `SKILL.md` against the Agent Skills format in CI (23.1).

- [ ] **TASK-30: Agent-skill evaluation**
  - **Associated Acceptance Criteria**: `23.2`, `23.3`
  - **Concrete Steps**:
    1. Write an evaluation script that installs the skill into a fixture Claude Code data folder, then runs real Claude Code headless against a fixture workspace with the two prompts in 23.2 and 23.3.
    2. Check the transcripts and the commands run: `plan` was used, go-ahead was asked for, no `mv`, `Move-Item` or `rename` was run, and the `--rewrite-sessions` command was handed to the user rather than applied.
    3. Run it only locally or on demand, since it needs an API key. Document how in the README's contributor section.
    4. Run it, and record the results in the walkthrough.

### Milestone 8: Ready to release

Once this is done, every criterion is proven across all three OSes, coverage gates are on, and the release workflow, install routes and README are ready.

- [ ] **TASK-31: Conformance suite across all adapters**
  - **Associated Acceptance Criteria**: `4.1`, `22.2`, `24.1`, `27.1`
  - **Concrete Steps**:
    1. Build a fixture set that exercises every adapter and action type. For each fixture, check that `plan`'s actions and counts equal those recorded by the following real run (4.1).
    2. Add a fixture producing each exit code in design.md's table, and check each against the documentation (22.2).
    3. Add a fictitious agent adapter in `awm-testkit` that moves its data with no change to `awm-core`'s source (24.1). Enforce that with a CI check on the diff.
    4. Check across all fixtures that every deletion was of an item the plan named as merged, copied-and-verified or temporary (27.1).

- [ ] **TASK-32: Privacy and no-network checks**
  - **Associated Acceptance Criteria**: `26.1`, `26.2`
  - **Concrete Steps**:
    1. Run the end-to-end suite on the Linux CI runner inside a network namespace with no interfaces (`unshare -n`), and fail on any attempted connection (26.1).
    2. Seed every fixture's transcripts, settings and process files with a sentinel secret string. Check that it never appears in any output, log, record or JSON from any command (26.2).
    3. Confirm `cargo-deny`'s ban on network client crates is in force.

- [ ] **TASK-33: Performance**
  - **Associated Acceptance Criteria**: `28.1`, `28.2`
  - **Concrete Steps**:
    1. Build a benchmark fixture generator: 100,000 workspace files and 2 GB of agent data shaped like real transcripts.
    2. Measure planning (no more than 15 seconds) and a same-volume move without opt-ins (no more than 30 seconds) on the CI Windows runner, and record the timings (28.1).
    3. Add progress output for any planning phase longer than 2 seconds (28.2).
    4. Profile and fix any phase over budget. If a limit can't be met, raise it with the owner rather than loosen it silently.

- [ ] **TASK-34: Cross-platform verification and coverage gates**
  - **Associated Acceptance Criteria**: `25.1`, `25.2`
  - **Concrete Steps**:
    1. Make the full suite a required check on Windows, macOS and Linux (25.1).
    2. Add the plan and report notice "verified against fixtures only" for adapters on an OS with no recorded real-world verification (25.2), driven by each adapter's declared verification.
    3. Turn on the coverage gates: per-module line and region thresholds on stable (90–100% for the critical modules listed at the top of this document, and the standard targets elsewhere), and branch thresholds in the nightly job.
    4. Fix gaps until the gates pass, or propose a justified lower target to the owner for any module where reaching it would take disproportionate effort.

- [ ] **TASK-35: Release workflow, install routes and README**
  - **Associated Acceptance Criteria**: none (release engineering, from tech.md's "Continuous integration and releases")
  - **Concrete Steps**:
    1. Add the release workflow: build binaries for x86_64 and aarch64 on Windows (MSVC), macOS and Linux (`musl`), and publish archives with SHA-256 checksums and build-provenance attestations to GitHub Releases. Publish the crate to crates.io.
    2. Add the install routes: `curl` and PowerShell install scripts that verify the checksum, a Homebrew tap, a Scoop bucket and a winget manifest.
    3. Write the README: what the tool does (linking product.md for the why), install routes, quick start, commands and flags, the opt-in rule, exit codes, the Smart App Control limitation until signing is in place, clearing macOS quarantine for browser downloads, and verified versions per adapter.
    4. Prepare the SignPath Foundation application, to submit at the first release (submitting it is the owner's decision), and leave a documented place in the workflow for its signing step.
