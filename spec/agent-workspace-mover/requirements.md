---
status: approved
---

# Requirements Specification: agent-workspace-mover, version 1

## 1. Project Summary

This spec covers version 1 of agent-workspace-mover: moving or renaming a workspace folder **on one machine**, carrying Claude Code's path-keyed data, git's links, absolute paths inside the workspace and VS Code's state along with it. It also covers rescuing data orphaned by an earlier move done by hand, scanning the machine for such data, finishing a move once whatever is holding the folder exits, machine-readable output, and an agent skill. What the product is, who it's for and why is in product.md and isn't repeated here.

Version 1 is verified on Windows 11 against Claude Code 2.1.309. macOS and Linux are supported and tested, but their adapters are labelled as checked against less real-world data until they've been used on real machines (REQ-25).

The requirements were drafted from product.md's "Version 1" goals, from the notes in the repo's `CLAUDE.md`, from the owner's reference script for the real move on 2026-10-22 (kept outside this repo), and from the resume experiment run on 2026-10-22 (section 3). Defaults proposed here that the owner hasn't yet confirmed are listed in section 4.

**Related documents**
- [Product](product.md)

## 2. Requirements (EARS Format)

Requirements use the Easy Approach to Requirements Syntax (EARS):
- **Ubiquitous**: The system shall <response>.
- **Event-Driven**: When <trigger>, the system shall <response>.
- **State-Driven**: While <precondition>, the system shall <response>.
- **Unwanted Behavior**: If <unwanted condition>, then the system shall <response>.
- **Optional Feature**: Where <feature is enabled>, the system shall <response>.

Terms used throughout:
- **Source** and **target**: the workspace folder's path before and after the move.
- **Agent data**: what an agent keeps outside the workspace, keyed on its path. For Claude Code, see REQ-6 and REQ-7.
- **The record**: what an agent has recorded about past work. This means a session's contents, and history kept outside the sessions, such as prompt history.
- **Opt-in**: one of the edits to the record in REQ-8, which happens only when the user asks for it by name for that run.
- **Under a path**: the path itself, or any path inside it. This compares whole path segments, so `C:\a\repo-old` is not under `C:\a\repo`.

### REQ-1: Move a workspace

**User Story**: As a developer who uses AI coding agents, I want to move or rename a workspace folder with one command, so that its agent data, version control and editor state come with it instead of being stranded.

- **1.0.1 (Event-Driven)**: When the user runs `move <source> <target>`, the system shall check everything (REQ-2), show the plan (REQ-3), and, once the user goes ahead, move the folder and carry every handled kind of path-keyed state across to the target.
- **1.0.2 (Ubiquitous)**: The system shall move a folder that is a git repository, a linked git worktree, a folder with no version control, or a folder inside a repository.
- **1.0.3 (State-Driven)**: While the source and target are on the same volume, the system shall move the folder with a single rename.
- **1.0.4 (State-Driven)**: While the source and target are on different volumes, the system shall copy the folder, verify the copy against the source (every file present, with matching size and content hash), and only then remove the source.
- **1.0.5 (Ubiquitous)**: The system shall carry across agent data, version-control links and editor state for every path under the source, so a folder holding several workspaces moves as one, with each workspace found listed in the plan.
- **1.0.6 (Event-Driven)**: When the move completes, the system shall verify the result (REQ-16) and report what it did, what it left unchanged by default, and anything it couldn't do, with the reason for each.

#### Acceptance Criteria
- **1.1**: Moving a git repository with three Claude Code sessions, memories and per-project settings produces the repository at the target, with all three sessions, the memories and the settings available there, and nothing left under the source's paths except what the report lists as deliberately left.
- **1.2**: Moving a folder with no version control succeeds and reports that no version-control adapter applied.
- **1.3**: Renaming a folder within its parent folder (same volume) performs one directory rename. No copy of the workspace appears anywhere during the move.
- **1.4**: Moving a workspace to another drive produces a byte-identical copy at the target, and the source is removed only after verification passes. A verification failure leaves the source intact and the run stopped, with the cause reported.
- **1.5**: Moving a folder containing two repositories, each with Claude Code sessions, carries both repositories' sessions, and the plan lists both.
- **1.6**: After any move, the report contains a "done", a "left unchanged" and a "couldn't do" section, each present even when empty.

### REQ-2: Check everything before changing anything

**User Story**: As a user trusting the tool with irreplaceable data, I want every problem caught before anything changes, so that a move never fails halfway through for a reason that could have been known in advance.

- **2.0.1 (Ubiquitous)**: The system shall run every check before making any change, and shall make no change if any check finds a blocker.
- **2.0.2 (Unwanted Behavior)**: If the target already exists, the source doesn't exist, the target is under the source, or the source is under the target, then the system shall report a blocker.
- **2.0.3 (Unwanted Behavior)**: If the source is a drive root, the user's home folder, a folder containing the user's home folder, or a folder holding agent data the tool manages (such as `~/.claude`), then the system shall refuse the move.
- **2.0.4 (Unwanted Behavior)**: If any process holds the source or anything under it (as its working folder, or by an open file that would prevent the move), then the system shall report a blocker naming each process by name and process id (REQ-20).
- **2.0.5 (Unwanted Behavior)**: If the plan would change an agent's data while that agent has any running process, then the system shall report a blocker listing the running processes (REQ-20).
- **2.0.6 (Unwanted Behavior)**: If an adapter finds data in a format, or from an agent version, that it doesn't recognise, then the system shall report a blocker for that adapter's changes, stating what was unrecognised (REQ-24).
- **2.0.7 (Unwanted Behavior)**: If any item to be carried to a path already exists there (a session file, a memory file, a settings entry), then the system shall report a blocker naming each collision. It shall not count an empty folder that would merge as a collision.
- **2.0.8 (Unwanted Behavior)**: If the user lacks permission to change anything in the plan, or a different-volume move needs more free space than the target volume has, then the system shall report a blocker.
- **2.0.9 (Ubiquitous)**: The system shall report every blocker it finds in one run, each with what the user can do about it, rather than stopping at the first.

#### Acceptance Criteria
- **2.1**: With the target already present, `move` exits with the "blocked" exit code (REQ-22), and every file under the source, the target and agent data is unchanged, byte for byte.
- **2.2**: `move C:\Users\<user> D:\x`, `move C:\ D:\x` and `move ~/.claude ~/elsewhere` are each refused, with a message saying why.
- **2.3**: With a terminal open in a subfolder of the source, the blockers name that terminal's process and id, and nothing changes.
- **2.4**: With a Claude Code process running and a plan that changes Claude Code data, the run is blocked and lists the process. A dry run in the same state succeeds (REQ-4).
- **2.5**: When a session file of the same name exists at both the source's and the target's agent data, the blockers name that file and nothing changes. An empty `memory/` folder at the target's agent data is not reported as a collision.
- **2.6**: A source with three different blockers reports all three in a single run.

### REQ-3: Show the plan and ask to go ahead

**User Story**: As a user about to move a workspace, I want to see exactly what will happen, with counts, before anything changes, so that I can decide with full information.

- **3.0.1 (Ubiquitous)**: The system shall present a plan listing every action it will take, grouped by adapter, with counts (for example, sessions moved, settings entries moved, worktrees repaired, files offered for path updates).
- **3.0.2 (Ubiquitous)**: The plan shall list what will be left unchanged by default, with counts, and name the opt-in or flag that would change each item.
- **3.0.3 (Ubiquitous)**: The plan shall say explicitly that per-project settings will move (REQ-7), listing which settings, and name the flag that leaves them behind.
- **3.0.4 (Ubiquitous)**: The plan shall list path-keyed state the tool detects but doesn't fix in version 1 (REQ-11).
- **3.0.5 (Event-Driven)**: When the plan has no blockers and the session is interactive, the system shall ask the user to go ahead and shall proceed only on an explicit yes.
- **3.0.6 (Optional Feature)**: Where the user passes `--yes`, the system shall proceed with the default move without asking. `--yes` shall never turn on an opt-in (REQ-8).
- **3.0.7 (State-Driven)**: While no one is at a terminal and `--yes` was not passed, the system shall print the plan and stop without changing anything.

#### Acceptance Criteria
- **3.1**: A plan for a repository with sessions, settings and a worktree shows a count for each of: session folders moved, session files, memory files, settings entries, worktrees repaired.
- **3.2**: A plan for a workspace whose sessions record the old path shows "left unchanged" items for recorded paths in sessions and prompt-history entries, with counts and the opt-in name for each.
- **3.3**: The plan names the settings that move (for example permissions, trust, MCP servers) and the `--fresh-settings` flag.
- **3.4**: Answering anything other than yes at the go-ahead prompt changes nothing, and exits with the "declined" exit code.
- **3.5**: `move --yes` run with no terminal completes the default move, and the report lists the opt-ins as not applied.
- **3.6**: `move` with no terminal and no `--yes` prints the plan, changes nothing and exits with the "declined" exit code.

### REQ-4: Dry run

**User Story**: As a user, I want to see the complete plan without changing anything, even while agents are running, so that I know what to close and what will happen.

- **4.0.1 (Event-Driven)**: When the user runs `move --dry-run` or `plan`, the system shall run every check and produce the full plan, and shall change nothing on disk except its own log.
- **4.0.2 (Ubiquitous)**: The dry run's list of actions and counts shall be identical to the actions and counts the real run would take from the same starting state.
- **4.0.3 (State-Driven)**: While agents are running or the folder is held, the dry run shall still complete, reporting those conditions as blockers for the real run.
- **4.0.4 (Ubiquitous)**: The dry run shall not open any file it reads with write access, and shall not create, rename or delete anything in the workspace, agent data or editor data.

#### Acceptance Criteria
- **4.1**: Over a test fixture set covering every adapter, the actions and counts of `plan` match those recorded by the following real run, for every fixture.
- **4.2**: After `plan`, a full listing with hashes of the workspace, the agent data and the editor data is identical to the listing before it.
- **4.3**: `plan` with a Claude Code process running completes, lists the process as a blocker, and exits with the "plan shown" exit code.

### REQ-5: Workspace and path model

**User Story**: As a user on any supported OS, I want the tool to recognise every form in which my workspace's path is recorded, so that nothing keyed on it is missed or wrongly matched.

- **5.0.1 (Ubiquitous)**: The system shall recognise a path in each form tools are known to record it: native, with forward slashes, JSON-escaped (`\\`), POSIX-style for Windows drives (`/c/Users/…`), and as a `file://` URI.
- **5.0.2 (State-Driven)**: While on Windows, the system shall compare paths without regard to case, including the drive letter. While on macOS, it shall follow the case sensitivity of the volume concerned.
- **5.0.3 (Ubiquitous)**: The system shall treat a path as matching only when it is the old path or under it (comparing whole segments), so that sibling folders whose names start with the same text are never matched.
- **5.0.4 (Ubiquitous)**: The system shall write a replaced path in the same form, separator style and drive-letter case convention as the text it replaces.
- **5.0.5 (Unwanted Behavior)**: If the source or target path contains a symbolic link, a junction or a subst drive, then the system shall resolve both the given and the resolved forms, match agent data recorded under either, and say in the plan which form it found.
- **5.0.6 (State-Driven)**: While on Windows, the system shall handle paths longer than 260 characters.

#### Acceptance Criteria
- **5.1**: Given agent data recording `C:\\Users\\u\\repo`, `c:/Users/u/repo`, `/c/Users/u/repo` and `file:///c:/Users/u/repo`, each is recognised as the source `C:\Users\u\repo`.
- **5.2**: Data recorded for `C:\Users\u\repo-old` is not matched when the source is `C:\Users\u\repo`.
- **5.3**: A replaced `/c/…` path stays in `/c/…` form, and a JSON-escaped path stays JSON-escaped.
- **5.4**: A source reached through a junction is moved, and agent data recorded under either its junction path or its real path is carried across.
- **5.5**: A workspace whose deepest file path exceeds 260 characters on Windows moves completely.

### REQ-6: Claude Code: session folders and memory

**User Story**: As a Claude Code user, I want my sessions, subagent transcripts and memories to come with the workspace, so that `claude --resume` and `claude --continue` in the new folder find everything the old one had.

- **6.0.1 (Ubiquitous)**: The system shall find every Claude Code project folder in Claude Code's data folder whose project path is the source or under it, including folders for worktrees under it and folders whose drive letter differs in case.
- **6.0.2 (Ubiquitous)**: The system shall decide a project folder's real path from the `cwd` values recorded in its transcripts, reading only, and use the encoded folder name only to find candidates, because the name's encoding is lossy (every character other than a letter or digit becomes `-`).
- **6.0.3 (Unwanted Behavior)**: If a project folder has no transcripts to read a path from, then the system shall match it only on an exact encoded name, and shall otherwise list it in the plan as ambiguous and leave it alone.
- **6.0.4 (Unwanted Behavior)**: If a project folder's transcripts record more than one project path that the encoding maps to its name, then the system shall report it as ambiguous and leave it alone.
- **6.0.5 (Ubiquitous)**: The system shall carry each matched project folder to the folder name Claude Code uses for the corresponding target path, by renaming it, or, when that folder already exists, by merging entries into it without overwriting any (REQ-2, 2.0.7).
- **6.0.6 (Ubiquitous)**: The system shall carry every entry in a project folder: transcripts, session folders (subagent transcripts, tool results), `memory/` and any entry it doesn't recognise, unchanged.
- **6.0.7 (Ubiquitous)**: The system shall honour Claude Code's configured data folder (`CLAUDE_CONFIG_DIR` where set) rather than assuming `~/.claude`.
- **6.0.8 (Unwanted Behavior)**: If the folder name Claude Code would use for the target can't be derived with confidence (for example, an encoded name long enough that Claude Code may shorten it), then the system shall report a blocker for this adapter rather than guess.
- **6.0.9 (Ubiquitous)**: The plan and report shall count, per project folder, the sessions Claude Code's `--resume` picker lists and those it doesn't (print-mode sessions, recorded with `entrypoint` `sdk-cli`, which can be reached only by id), so that a user doesn't mistake sessions the picker never showed for sessions the move lost.

#### Acceptance Criteria
- **6.1**: After a move, `claude --continue` in the target folder continues the most recent session started in the source folder (verified for 2.1.309 by the experiment in section 3).
- **6.2**: After a move, the interactive `claude --resume` picker in the target folder lists every session it listed in the source folder before the move, including the worktrees' sessions in their own target folders.
- **6.3**: Subagent transcripts, tool results and memory files arrive in the target project folder with identical content hashes.
- **6.4**: With an empty target project folder already present (holding only `memory/`), the move merges into it and the run succeeds.
- **6.5**: A project folder for the sibling `…\repo-old` is not touched when moving `…\repo`, even though its encoded name starts with the source's encoded name.
- **6.6**: With `CLAUDE_CONFIG_DIR` pointing at a fixture folder, the tool reads and changes only that folder.
- **6.7**: No transcript's content changes during a default move, verified by content hash.
- **6.8**: For a project folder holding two interactive sessions and one print-mode session, the plan reports two sessions the picker lists and one reachable only by id.

### REQ-7: Claude Code: per-project settings

**User Story**: As a Claude Code user, I want my permissions, trust and MCP choices to come with the workspace by default, with a clear way to start fresh, so that the agent doesn't treat the moved workspace as a stranger unless I want it to.

- **7.0.1 (Ubiquitous)**: By default, the system shall carry each `projects` entry in Claude Code's global config (`~/.claude.json`) whose key is the source or under it to the matching target key, with its value unchanged.
- **7.0.2 (Ubiquitous)**: The system shall update each path under the source in `githubRepoPaths` to the matching target path, whether or not settings move, because it records where a clone lives rather than a setting.
- **7.0.3 (Optional Feature)**: Where the user passes `--fresh-settings`, the system shall leave the `projects` entries under their old keys, untouched, and the report shall say so and that `scan` can find them.
- **7.0.4 (Ubiquitous)**: The system shall edit the global config as JSON: parse it, change only the keys and values named in the plan, preserve every other key and value and its order, and write it back atomically.
- **7.0.5 (Unwanted Behavior)**: If the global config fails to parse, or changed between planning and writing, then the system shall not write it, and shall report this and leave the rest of the plan's state consistent (REQ-13).

#### Acceptance Criteria
- **7.1**: After a default move, the target's `projects` entry holds the source entry's `allowedTools`, `hasTrustDialogAccepted`, `mcpServers` and `enabledMcpjsonServers` values, unchanged, and the source key is gone.
- **7.2**: After a move with `--fresh-settings`, the source key's entry is unchanged and there is no target key. The report says the old settings were left behind.
- **7.3**: After a move, `githubRepoPaths` lists the target path in place of the source path, in both the default and `--fresh-settings` cases.
- **7.4**: Parsing the config before and after a move shows differences only in the keys and values named in the plan.
- **7.5**: A config modified by another process between planning and writing is not overwritten, and the run reports it.

### REQ-8: Editing the record is opt-in

**User Story**: As a user whose agent sessions are a record of past work, I want the tool to leave that record as it is unless I explicitly ask, so that nothing about what happened is changed behind my back.

- **8.0.1 (Ubiquitous)**: The system shall not change the contents of any session, or any history kept outside the sessions, unless the user opts in to that specific edit for that run.
- **8.0.2 (Ubiquitous)**: Version 1 shall offer two opt-ins for Claude Code: `rewrite sessions` (the `cwd` field recorded on transcript lines, in every recognised path form) and `rewrite history` (the `project` field in `~/.claude/history.jsonl`).
- **8.0.3 (Event-Driven)**: When the user requests an opt-in by its flag, the system shall show exactly what it will change, with counts per file, and then require the user to type the opt-in's exact phrase (`rewrite sessions`, `rewrite history`), compared case-sensitively. Any other input skips the opt-in.
- **8.0.4 (Unwanted Behavior)**: If no one is at a terminal to type the phrase (input or output isn't a terminal), then the system shall not apply the opt-in, and shall report the command the user can run themselves.
- **8.0.5 (Ubiquitous)**: The system shall not accept an opt-in's confirmation from a config file, an environment variable, a command-line argument, piped input or `--yes`.
- **8.0.6 (Ubiquitous)**: An opt-in shall change only the named field's value, only where the value is the old path or under it, and shall leave conversation text, other fields and every other path unchanged.
- **8.0.7 (Ubiquitous)**: The system shall not change `serverClassifierContext.context.live_cwd`, `serverClassifierContext.context.git_state.root`, or any other field the adapter doesn't name as part of an opt-in, even when an opt-in is applied.
- **8.0.8 (Ubiquitous)**: The system shall back up every file before an opt-in changes it (REQ-12).

#### Acceptance Criteria
- **8.1**: A default move leaves every transcript and `history.jsonl` unchanged, verified by content hash.
- **8.2**: With `--rewrite-sessions`, typing `rewrite sessions` changes only `cwd` values. A transcript whose conversation text contains the old path keeps that text unchanged.
- **8.3**: Typing `Rewrite sessions`, `yes`, `y` or nothing skips the opt-in, and the report lists it as not applied.
- **8.4**: Running with `--rewrite-history` and piped input (`echo "rewrite history" | …`) does not apply the opt-in, and reports the command to run in a terminal.
- **8.5**: No config key or environment variable exists that applies an opt-in. A test enumerating the tool's config schema and the environment variables it reads finds none.
- **8.6**: After `rewrite sessions`, `live_cwd` and `git_state.root` values are unchanged.

### REQ-9: Version control

**User Story**: As a developer, I want version control's links to stay healthy after a move, so that worktrees and repositories keep working without manual repair.

- **9.0.1 (Ubiquitous)**: The system shall detect which version-control adapter applies to the source, and move a folder with none, reporting that none applied.
- **9.0.2 (Event-Driven)**: When the source is a git repository with linked worktrees (inside it or elsewhere), the system shall repair the links in both directions after the move.
- **9.0.3 (Event-Driven)**: When the source is itself a linked worktree, the system shall repair the link from the main repository to its new location.
- **9.0.4 (Event-Driven)**: When the source contains git submodules, or a repository whose config or `.git` files record absolute paths under the source, the system shall detect each absolute path and report it in the plan, with what to do about it.
- **9.0.5 (Ubiquitous)**: The system shall not change remotes or anything held by a server. Where a remote's name suggests it may need attention (for example, a repository renamed to a name its remote doesn't match), the system shall report it.
- **9.0.6 (Unwanted Behavior)**: If git isn't installed, or a repair fails, then the system shall report it as a "couldn't do" item, with the command that would fix it.

#### Acceptance Criteria
- **9.1**: After moving a repository with worktrees under `.claude/worktrees/` and one worktree outside the repository, `git worktree list` in the target lists every worktree at its correct path with no "prunable" entries, and `git status` succeeds in each.
- **9.2**: After moving a linked worktree, `git worktree list` in the main repository shows its new path.
- **9.3**: A repository with a submodule whose config records an absolute path under the source shows that path in the plan.
- **9.4**: With git missing from `PATH`, a repository with worktrees still moves, and the report gives the `git worktree repair` command to run.

### REQ-10: Absolute paths inside the workspace

**User Story**: As a developer, I want absolute paths inside my workspace that point at its old location to be found and offered for update, so that configs and scripts don't silently point nowhere.

- **10.0.1 (Ubiquitous)**: The system shall search the workspace's text files for the source path in every recognised form (REQ-5), skipping `.git`, binary files, and files ignored by the workspace's version control.
- **10.0.2 (Ubiquitous)**: The system shall never update a path inside the workspace without the user selecting it.
- **10.0.3 (Event-Driven)**: When matches are found in an interactive run, the system shall present them by file, with the number of matches and a preview of each matched line (with values that look like secrets masked), and let the user choose which files to update.
- **10.0.4 (Ubiquitous)**: The system shall leave files that look like records unselected by default: changelogs, logs, release notes, transcripts and lock files.
- **10.0.5 (Optional Feature)**: Where the user passes an explicit list of files to update (`--update-paths <file…>`), the system shall update the matches in those files without prompting, and only those.
- **10.0.6 (Optional Feature)**: Where the user passes `--include-ignored`, the system shall also search files ignored by version control.
- **10.0.7 (Ubiquitous)**: The system shall back up each file before updating it, and shall preserve its encoding, line endings and byte-order mark.

#### Acceptance Criteria
- **10.1**: A workspace with the source path in `config.json` (JSON-escaped), `run.sh` (POSIX form) and `CHANGELOG.md` produces three matched files, with `CHANGELOG.md` unselected and the other two selected.
- **10.2**: A non-interactive move with no `--update-paths` reports the matches and changes none of them.
- **10.3**: `--update-paths config.json` updates only `config.json`. Its line endings and byte-order mark are unchanged, and a backup exists.
- **10.4**: Paths under `.git`, inside a binary file or inside an ignored `node_modules` are not reported unless `--include-ignored` is given (and `.git` never is).

### REQ-11: Other path-keyed state: detect and report

**User Story**: As a developer, I want to be told about other tools whose state a move will break, even where version 1 can't fix them, so that I'm not surprised later.

- **11.0.1 (Ubiquitous)**: The system shall detect, and report in the plan with what the user can do about each: Python virtual environments inside the workspace (`pyvenv.cfg`), Pipenv and Poetry environments kept outside it for this workspace, Docker Compose projects whose name comes from the folder name, direnv and mise allow or trust entries keyed on paths under the source, and JetBrains IDE per-project state kept outside the workspace.
- **11.0.2 (Ubiquitous)**: The system shall not change any of the state in 11.0.1 in version 1.
- **11.0.3 (Unwanted Behavior)**: If a detector can't read the state it looks for, then the system shall report that the check couldn't be done, rather than report nothing.
- **11.0.4 (Ubiquitous)**: Each detector shall state in the report whether it has been verified on the current OS (REQ-25).

#### Acceptance Criteria
- **11.1**: A workspace containing `.venv/pyvenv.cfg` produces a plan item naming the environment and saying it must be recreated or repaired.
- **11.2**: A workspace with `compose.yaml` and no `name:` produces a plan item saying the Compose project's name will change and how to keep it (`name:` or `COMPOSE_PROJECT_NAME`).
- **11.3**: A direnv allow entry for `<source>/.envrc` is reported, with the `direnv allow` command to run after the move.
- **11.4**: No file belonging to these tools changes during a move, verified by content hash.

### REQ-12: Backups and the move record

**User Story**: As a user, I want everything the tool changes to be backed up and every move recorded, so that nothing is ever lost and any move can be reversed.

- **12.0.1 (Ubiquitous)**: The system shall copy every file it will change to a backup, and verify the copy, before changing it.
- **12.0.2 (Ubiquitous)**: The system shall keep, for each move, a record of the plan as approved and of each step as it starts and completes, written to stable storage before the step's effects, so that an interruption at any point leaves enough to resume or roll back (REQ-13).
- **12.0.3 (Ubiquitous)**: The system shall write every changed file atomically (write a new file, flush it, then replace the old one), so that a file is never left half-written.
- **12.0.4 (Ubiquitous)**: The system shall keep backups and records on the local machine only, readable only by the user who ran it.
- **12.0.5 (Ubiquitous)**: The system shall keep backups until the user removes them with an explicit command (`backups prune`, naming a move or an age), and shall never remove them automatically.
- **12.0.6 (Event-Driven)**: When the user runs `history` (or `backups list`), the system shall list past moves with their date, source, target, state and backup size.

#### Acceptance Criteria
- **12.1**: For every file changed during a move, a backup with a matching hash of its prior content exists before the change is applied (verified by injecting a failure immediately after each backup).
- **12.2**: Killing the process at each step boundary of a fixture move leaves a record from which `resume` or `rollback` succeeds (REQ-13, 13.1).
- **12.3**: On Windows, backup and record files grant access only to the user who ran the tool. On macOS and Linux, their mode is `0600` (folders `0700`).
- **12.4**: Backups from a move 90 days old still exist until `backups prune` removes them.

### REQ-13: Resume or roll back an interrupted move

**User Story**: As a user whose move was interrupted (by a crash, a power cut or a failed step), I want to finish it or put everything back, so that I'm never left with half a move.

- **13.0.1 (Event-Driven)**: When the tool starts and finds an incomplete move, the system shall report it and refuse to start another move involving the same paths until it's resumed or rolled back.
- **13.0.2 (Event-Driven)**: When the user runs `resume`, the system shall re-run the checks for the remaining steps and complete them.
- **13.0.3 (Event-Driven)**: When the user runs `rollback`, the system shall reverse the completed steps in reverse order, restoring changed files from their backups.
- **13.0.4 (Event-Driven)**: When a step fails during a move, the system shall stop, leave completed steps in place, and report the failure with the `resume` and `rollback` commands.

#### Acceptance Criteria
- **13.1**: For each step boundary of a fixture move, interrupting there and running `rollback` restores the workspace, agent data and editor data to a state matching the pre-move listing with hashes. Interrupting there and running `resume` produces a state matching an uninterrupted move.
- **13.2**: Starting a new move of the same source while a move is incomplete is refused, and the message names `resume` and `rollback`.

### REQ-14: Undo

**User Story**: As a user who regrets a move, I want to reverse it, so that everything is as it was before.

- **14.0.1 (Event-Driven)**: When the user runs `undo`, the system shall reverse the most recent completed move. When given a move's id, it shall reverse that move.
- **14.0.2 (Ubiquitous)**: Undo shall run the same checks, plan and go-ahead as a move (REQ-2, REQ-3), and shall support `--dry-run`.
- **14.0.3 (Unwanted Behavior)**: If data has been added or changed at the target since the move (for example, a new session, a changed memory file or changed settings), then undo shall carry the new data back with the rest, and shall report a blocker for any file both changed since the move and also needing restoration from a backup, rather than overwrite either.
- **14.0.4 (Unwanted Behavior)**: If a move's backups have been pruned, then undo shall still reverse renames and moves that need no backup, and shall report, before going ahead, each content change it can no longer reverse.
- **14.0.5 (Unwanted Behavior)**: If a later move involves the same paths, then undo of the earlier move shall be refused until the later one is undone.

#### Acceptance Criteria
- **14.1**: Move, then undo, gives a listing with hashes of the workspace, agent data and editor data identical to the listing before the move, apart from the tool's own records.
- **14.2**: Move, start a new Claude Code session in the target, then undo, puts the new session in the source's project folder, along with the original sessions.
- **14.3**: Undo of a move whose opt-in `rewrite sessions` was applied restores the transcripts' original content from the backups.
- **14.4**: Undo of an earlier move, when a later move involved the same workspace, is refused, naming the later move.

### REQ-15: Rescue a move that already happened

**User Story**: As a user who moved a workspace by hand and lost its sessions, I want to reattach the orphaned agent data to where the workspace is now, so that I get my history back.

- **15.0.1 (Event-Driven)**: When the user runs `rescue <old-path> <current-path>`, the system shall plan and carry the agent data and settings keyed on the old path to the current path, as a move would for everything except the folder itself.
- **15.0.2 (Event-Driven)**: When the user runs `rescue <current-path>` without an old path, the system shall look for orphaned agent data that matches the workspace's identity (REQ-17), present the candidates with the evidence and a confidence level, and let the user choose.
- **15.0.3 (Ubiquitous)**: Rescue shall never attach data to a workspace without the user choosing the candidate, interactively or by naming the old path.
- **15.0.4 (Ubiquitous)**: Rescue shall follow the same rules as a move: checks, plan, go-ahead, backups, opt-ins, record and undo.

#### Acceptance Criteria
- **15.1**: After moving a fixture repository by hand, `rescue <old> <new>` makes its sessions available from the new folder (as in 6.1 and 6.2), and moves its settings.
- **15.2**: `rescue <new>` for that repository lists the orphaned project folder as a candidate, with its evidence (for example, a matching remote) and a confidence level.
- **15.3**: `rescue` with no terminal and no old path lists the candidates and changes nothing.
- **15.4**: `undo` after a rescue restores the orphaned state.

### REQ-16: Verify and look for leftovers

**User Story**: As a user, I want the tool to prove a move is complete rather than assume it is, so that a store nobody knew about doesn't silently keep the old path.

- **16.0.1 (Event-Driven)**: When a move, rescue or undo completes, the system shall verify each adapter's result: for Claude Code, that the target's project folder holds the expected number of transcripts and that settings sit under the target key; for git, that `git worktree list` reports no broken worktrees.
- **16.0.2 (Event-Driven)**: When a move completes, the system shall search Claude Code's data folder, its global config and VS Code's data for remaining occurrences of the source path, and report the number of occurrences per file and, for JSON files, the names of the fields they appear in, never the values.
- **16.0.3 (Ubiquitous)**: The leftover search shall separate leftovers that are expected (the record left unchanged by default, settings left behind with `--fresh-settings`) from unexpected ones, and shall report unexpected ones as warnings.

#### Acceptance Criteria
- **16.1**: After a default move, leftovers report the counts of recorded paths in transcripts and in `history.jsonl` as expected, and nothing as unexpected.
- **16.2**: A fixture with an extra, unknown file in Claude Code's data folder containing the source path produces an unexpected-leftover warning naming the file and the count, and the exit code for "succeeded with warnings".
- **16.3**: No leftover report contains any text from a file other than its path, field names and counts.

### REQ-17: Workspace identity

**User Story**: As a user rescuing or scanning, I want the tool to recognise a workspace by what it is, not only by where it is, so that it can match orphaned data to wherever the workspace lives now.

- **17.0.1 (Ubiquitous)**: The system shall compute a git workspace's identity from its root commit or commits and its remote URLs, normalised (scheme, credentials and `.git` suffix removed).
- **17.0.2 (Ubiquitous)**: The system shall gather identity evidence for orphaned agent data from what the agent recorded, such as `githubRepoPaths` keys, the last segment of the recorded path, and git branch names recorded in transcripts, reading only.
- **17.0.3 (Ubiquitous)**: The system shall rate each candidate match as high, medium or low confidence, and show the evidence behind each rating.
- **17.0.4 (Ubiquitous)**: For a folder with no version control, the system shall match on recorded path and folder name only, and rate such matches low.
- **17.0.5 (Ubiquitous)**: The system shall strip credentials from any remote URL before storing or showing it.

#### Acceptance Criteria
- **17.1**: Two clones of the same repository at different paths have equal identities. Two unrelated repositories don't.
- **17.2**: An orphaned project folder whose `githubRepoPaths` entry is `owner/repo` matches a current clone whose remote is `https://github.com/owner/repo.git` as high confidence.
- **17.3**: A remote URL containing `user:token@` is never shown, logged or stored with the credentials.

### REQ-18: Scan the machine

**User Story**: As a user, I want to see which agent data no longer has a folder, so that I can rescue it or deliberately clean it up.

- **18.0.1 (Event-Driven)**: When the user runs `scan`, the system shall list agent data and settings entries whose project path no longer exists, with each one's size, session count, date of last activity, and likely current locations (REQ-17).
- **18.0.2 (Ubiquitous)**: The scan shall group data whose path was a temporary folder separately, as likely leftovers.
- **18.0.3 (Ubiquitous)**: The scan shall be read-only, safe to run while agents run, and shall never delete anything. It may print the command the user could run to rescue an entry.
- **18.0.4 (Optional Feature)**: Where the user names folders to search (`--search <folder…>`), the scan shall look for likely current locations there, and otherwise only in the parent folders of the recorded paths and the user's home folder to a limited depth.

#### Acceptance Criteria
- **18.1**: A fixture with two orphaned project folders, one of which has a matching clone elsewhere, lists both, with the match and a `rescue` command for the one that has it.
- **18.2**: A project folder for a path under the system temporary folder is listed under leftovers.
- **18.3**: A listing with hashes of all agent data is identical before and after `scan`.

### REQ-19: Finish a blocked move later

**User Story**: As a user whose folder is held by an editor, a terminal or the agent session that asked for the move, I want the move to finish once they exit, so that I don't have to come back to it.

- **19.0.1 (Event-Driven)**: When a move is blocked only by processes holding the source or by running agents, and the user asks for it to finish later (`--when-free`), the system shall record the approved plan and wait, without needing administrator rights, for the blocking processes to exit.
- **19.0.2 (Ubiquitous)**: The waiting process shall survive the terminal or agent session that started it closing.
- **19.0.3 (Event-Driven)**: When every blocking process has exited, the system shall re-run all checks and compute the plan again. It shall go ahead only if the new plan has no blocker and no kind of action that the approved plan didn't have. Otherwise it shall stop and record why.
- **19.0.4 (Ubiquitous)**: A move finished later shall not apply any opt-in. Its report shall give the command for each opt-in the user can run afterwards.
- **19.0.5 (Event-Driven)**: When the user runs `status`, the system shall show any waiting move, what it waits for, and the outcome of the last move finished later. When the user runs `cancel`, it shall stop waiting and change nothing.
- **19.0.6 (Unwanted Behavior)**: If the blocking processes haven't exited within the waiting limit (24 hours by default, configurable), then the system shall stop waiting, change nothing, and record that it gave up.
- **19.0.7 (Ubiquitous)**: The system shall allow at most one waiting move per source.

#### Acceptance Criteria
- **19.1**: With a fixture process holding the source, `move --when-free` returns at once. Ending the process causes the move to complete within 30 seconds, and `status` shows it succeeded.
- **19.2**: A waiting move started from a terminal that is then closed still completes when the holder exits.
- **19.3**: If a new collision appears while it waits, the move doesn't happen, and `status` names the collision.
- **19.4**: `cancel` stops the waiting process, and nothing changes.
- **19.5**: A move finished later with `--rewrite-sessions` requested doesn't apply the opt-in, and its report gives the command.

### REQ-20: Locks, running processes and the tool's own session

**User Story**: As a user on Windows especially, I want to be told exactly what is holding a folder or using agent data, including the agent session I'm talking to, so that I know what to close.

- **20.0.1 (Ubiquitous)**: The system shall identify processes whose working folder is the source or under it, and processes with files open under it, on every supported OS.
- **20.0.2 (Ubiquitous)**: The system shall identify running processes of each supported agent.
- **20.0.3 (Event-Driven)**: When the tool itself, or a process it was launched from, is one of the holders (for example, the agent session that asked for the move), the system shall say so plainly, and offer to finish the move later (REQ-19) or give the one command to run after that session ends.
- **20.0.4 (Unwanted Behavior)**: If the system can't tell what is holding a folder, then it shall say so, and report the move as at risk of being blocked rather than claim it's free.
- **20.0.5 (Ubiquitous)**: The system shall read no more of an agent's process files than it needs to identify processes, and shall never print their contents.

#### Acceptance Criteria
- **20.1**: On Windows, a `cmd.exe` whose working folder is under the source, and a process holding a file open under it, are both named with their process ids.
- **20.2**: Run from a Claude Code session whose working folder is the source, the plan says that session is holding the folder and offers `--when-free`.
- **20.3**: No output, log or record contains the contents of `~/.claude/sessions/*.json`.

### REQ-21: VS Code state

**User Story**: As a VS Code user, I want the editor's recent folders and per-workspace state to follow the workspace, so that the editor opens it where it is now, with its state intact.

- **21.0.1 (Ubiquitous)**: The system shall update the source's entries to the target in VS Code's recently-opened list (the `history.recentlyOpenedPathsList` value in `globalStorage/state.vscdb`, a SQLite database), and in `globalStorage/storage.json` (window state and backup workspaces).
- **21.0.2 (Ubiquitous)**: For each workspace storage folder (`workspaceStorage/<id>/`) whose `workspace.json` names the source or a folder under it, the system shall move that folder to the id VS Code computes for the corresponding target folder, and update its `workspace.json`. VS Code computes the id from the folder's path and its creation time (its inode on Linux), so a folder left under the old id is never found again.
- **21.0.3 (Unwanted Behavior)**: If VS Code is running, then the system shall report a blocker for VS Code's changes, since it may write its state back.
- **21.0.4 (Ubiquitous)**: The system shall edit VS Code's JSON as JSON (as in 7.0.4), including JSON stored as a value in `state.vscdb`. It shall change only the planned rows of the database, within a single transaction. It shall support the stable release only. Other editions it finds (such as Insiders or VSCodium) shall be reported as detected but not handled.
- **21.0.5 (Unwanted Behavior)**: If the system's id calculation doesn't reproduce an existing workspace storage folder's id from its source folder, then the system shall not re-key any VS Code workspace storage, and shall report the calculation as unrecognised (as in 24.0.3).
- **21.0.6 (Unwanted Behavior)**: If a workspace storage folder already exists at the target's id (because VS Code has already opened the target), then the system shall report a collision blocker naming both folders (as in 2.0.7), rather than merge two workspaces' state.
- **21.0.7 (Ubiquitous)**: The system shall compute each target id from the target folder as it exists after the move. On a move across volumes, it shall carry the folder's creation time across where the OS allows (Windows, macOS), so that the id stays valid.

#### Acceptance Criteria
- **21.1**: After a move, VS Code's recent list shows the target and not the source, and opening the target restores the workspace's state stored before the move (for example, its open editors' layout and its scoped settings).
- **21.2**: With VS Code running, the plan lists it as a blocker for VS Code's changes.
- **21.3**: Parsing VS Code's JSON before and after (including the JSON value in `state.vscdb`) shows differences only in the entries named in the plan. Every other row of `state.vscdb` is unchanged.
- **21.4**: A fixture whose workspace storage folder name doesn't match the id calculated for its folder blocks VS Code's re-keying, and the plan names the folder without printing its contents.
- **21.5**: With a workspace storage folder already present at the target's id, the plan reports a collision naming both folders, and nothing in VS Code's data changes.
- **21.6**: After a move across volumes on Windows, the target folder's creation time equals the source's, and its workspace storage sits under the id VS Code computes for it.

### REQ-22: Machine-readable output and exit codes

**User Story**: As an agent, a script or the tool's own skill, I want structured plans and reports with stable exit codes, so that I can drive the tool reliably.

- **22.0.1 (Optional Feature)**: Where the user passes `--json`, the system shall write the plan, report, scan and status as JSON to standard output, matching a published schema that carries a version number, and send any human-readable messages to standard error.
- **22.0.2 (Ubiquitous)**: The system shall use documented, stable exit codes that distinguish at least: success; succeeded with warnings or items left; plan shown or declined; blocked by checks; failed and rolled back; failed and needing attention (incomplete); and usage error.
- **22.0.3 (Ubiquitous)**: JSON output shall never contain secrets or text from agent data, under the same rules as human-readable output (REQ-26).
- **22.0.4 (Ubiquitous)**: A schema change that would break existing readers shall come with a new schema version number.

#### Acceptance Criteria
- **22.1**: `plan --json` output validates against the published schema for every fixture.
- **22.2**: Each exit code listed in 22.0.2 is produced by at least one test fixture, and matches the documentation.
- **22.3**: `--json` output contains nothing on standard output apart from one JSON document.

### REQ-23: Agent skill

**User Story**: As a user who asks my agent to rename or move a workspace, I want the agent to use this tool safely, so that the agent doesn't use `mv` and strand its own history.

- **23.0.1 (Ubiquitous)**: The system shall ship a `SKILL.md` in the open Agent Skills format, installable for Claude Code, describing when and how to use the tool.
- **23.0.2 (Ubiquitous)**: The skill shall instruct the agent to: use the tool instead of moving the folder itself; run `plan --json` and explain the plan to the user; get the user's go-ahead before running `move --yes`; never request an opt-in unless the user asked for that edit in so many words, and then hand the user the command to run in their own terminal; and recognise its own session as a holder and finish with `--when-free` or the one command to run afterwards.
- **23.0.3 (Ubiquitous)**: The skill shall tell the agent how to spot orphaned sessions from an earlier move done by hand (via `scan` or `rescue <current-path>`), and offer to rescue them.
- **23.0.4 (Event-Driven)**: When the user runs the tool's `skill install` command, the system shall show where it will install the skill and install it only after the user goes ahead.

#### Acceptance Criteria
- **23.1**: The shipped `SKILL.md` passes the Agent Skills format's validation.
- **23.2**: In a scripted evaluation, Claude Code with the skill installed, asked to "rename this repo to X", runs `plan` and asks for go-ahead, and doesn't run `mv`, `Move-Item` or `rename`.
- **23.3**: In the same evaluation, asked to "move it and fix the session paths", the agent hands the user the `--rewrite-sessions` command, rather than trying to apply the opt-in itself.

### REQ-24: Adapters and agent versions

**User Story**: As a maintainer or contributor, I want each agent's, version-control system's and editor's knowledge contained in one adapter that declares what it has been verified against, so that adding support is a contained job and unknown formats are never guessed at.

- **24.0.1 (Ubiquitous)**: The system shall keep each agent's, each version-control system's and each editor's knowledge in its own adapter, behind interfaces the core uses for every adapter. The core shall contain nothing specific to any one of them.
- **24.0.2 (Ubiquitous)**: Each agent adapter shall declare the agent versions and OSes it has been verified against, and shall detect the installed version and the version that wrote the data it reads, where these can be found.
- **24.0.3 (Unwanted Behavior)**: If the installed agent version, or the format of its data, is outside what the adapter recognises, then the system shall refuse to change that agent's data. It shall allow a dry run and report what it found, and may allow the change only with an explicit flag (`--allow-unverified <adapter>`) that the report records.
- **24.0.4 (Ubiquitous)**: The report shall state, per adapter, the verified versions and whether the current version and OS are among them.

#### Acceptance Criteria
- **24.1**: A test adapter for a second, fictitious agent can be added, and its data is moved, without any change to the core's source files.
- **24.2**: With Claude Code's reported version changed to one outside the adapter's declared range, a move that would change Claude Code data is blocked, and `plan` reports the version found.
- **24.3**: A transcript line in an unrecognised format blocks Claude Code's changes, and the plan names the file and line number without printing the line.

### REQ-25: Platforms

**User Story**: As a user on Windows, macOS or Linux, I want the tool to work on my machine and to tell me honestly how well tested it is there.

- **25.0.1 (Ubiquitous)**: The system shall run on Windows 10 and 11, current macOS, and current major Linux distributions, as a command-line tool that needs no administrator rights for any v1 feature.
- **25.0.2 (Ubiquitous)**: Version 1 shall be verified on Windows against real data, and tested on macOS and Linux against fixtures and in CI.
- **25.0.3 (State-Driven)**: While running on an OS for which an adapter's facts haven't been verified against real use, the system shall say so in the plan and report.

#### Acceptance Criteria
- **25.1**: The full automated test suite passes in CI on Windows, macOS and Linux.
- **25.2**: On macOS or Linux, the plan shows an "adapter verified against fixtures only" notice for the Claude Code adapter until that's changed by a recorded real-world verification.

### REQ-26: Privacy and secrets

**User Story**: As a user whose sessions hold code, conversations and sometimes secrets, I want the tool never to leak them, so that using it carries no privacy risk.

- **26.0.1 (Ubiquitous)**: The system shall make no network requests and collect no telemetry.
- **26.0.2 (Ubiquitous)**: The system shall not print, log or store the contents of agent data, beyond paths, field names and counts, except a masked preview of matches inside the workspace (REQ-10).
- **26.0.3 (Ubiquitous)**: The system shall treat backups as sensitive: kept local, readable only by the user (REQ-12), and never included in output.

#### Acceptance Criteria
- **26.1**: A move run under a network-blocking test harness succeeds with no attempted connections.
- **26.2**: In a fixture whose transcripts and process files contain a sentinel secret string, the string doesn't appear in any output, log, record or JSON from any command.

### REQ-27: Safety invariants

**User Story**: As a user, I want guarantees about what the tool will never do, so that I can trust it with data I can't replace.

- **27.0.1 (Ubiquitous)**: The system shall never overwrite an existing file, folder or settings entry it didn't create in the same run, except a file it changes as planned after backing it up.
- **27.0.2 (Ubiquitous)**: The system shall delete only what a planned step makes redundant: a folder emptied by a merge, a source verified copied to another volume, or its own temporary files.
- **27.0.3 (Ubiquitous)**: The system shall never delete agent data, settings or backups as a cleanup step, and shall never offer to.
- **27.0.4 (Ubiquitous)**: The system shall not report a move as successful unless every planned step completed and verification passed. Otherwise it shall report what's incomplete.

#### Acceptance Criteria
- **27.1**: Across all fixtures, every deletion the tool makes is of an item named in the plan as merged, copied-and-verified or temporary.
- **27.2**: A fixture where verification fails after all steps complete reports "needing attention", not success, and exits with that code.

### REQ-28: Performance

**User Story**: As a user, I want a typical move to take seconds, so that the tool is quicker than doing it by hand.

- **28.0.1 (Ubiquitous)**: For a workspace of up to 100,000 files with up to 2 GB of agent data, the system shall produce a plan within 15 seconds, and complete a same-volume move, excluding opt-ins, within 30 seconds, on a typical developer machine with an SSD.
- **28.0.2 (Ubiquitous)**: During planning, the system shall show progress for any phase that takes longer than 2 seconds.

#### Acceptance Criteria
- **28.1**: A benchmark fixture at those sizes meets both limits on the CI Windows runner, with the timings recorded.
- **28.2**: Planning a 100,000-file workspace shows progress output before the plan.

## 3. Evidence behind these requirements

**Resume experiment, 2026-10-22, Claude Code 2.1.309, Windows 11.** A throwaway folder A had one print-mode session (`claude -p`, with a fixed session id and a codeword). A was moved to B. Then, from B:

| Test | Session folder still named for A | Session folder merged into B's |
|---|---|---|
| `claude -c` (most recent in this folder) | not found | found |
| `claude --resume <id>` | found | found |
| `claude --resume` picker (checked by the owner) | — | not listed |

The picker didn't list the session, so three labelled copies were added to B's project folder, varying the two fields that might explain it:

| Copy | `cwd` recorded | `entrypoint` | Listed by the picker |
|---|---|---|---|
| original | A | `sdk-cli` | no |
| V1 | B | `sdk-cli` | no |
| V2 | A | `cli` | **yes** |
| V3 | B | `cli` | **yes** |

**Conclusion:** the picker hides print-mode sessions (`entrypoint` `sdk-cli`), and doesn't look at the recorded `cwd`. A session is found by the project folder it's in. Renaming the session folder is enough to make sessions resumable from the new folder, in the picker and with `-c`. Rewriting `cwd` isn't needed, so it can stay an opt-in without making the default move incomplete. This answers product.md's open question "Does a session still resume when only its folder is renamed?" for Claude Code 2.1.309 on Windows. Other findings:
- Looking up a session by id isn't tied to the folder.
- Print-mode sessions are common: 84 of the 192 transcripts on this machine begin as `sdk-cli`. A user will see fewer sessions in the picker than the folder holds, before a move as well as after it (6.0.9).
- Running any Claude Code session in a folder, even with `--no-session-persistence`, creates that folder's project folder with an empty `memory/` inside. So the target's project folder will often already exist, and merging is the normal case (6.0.5, 6.4).
- A print-mode session wrote nothing to `~/.claude.json` or `history.jsonl`, and recorded `cwd` only in the Windows form.

**Not covered by the experiment, still to verify:** V2 and V3 were edited copies, not real interactive sessions, so a real one should confirm the result (6.2). Also still to check: the POSIX form of `cwd` that interactive sessions record, worktrees' project folders, encoded names longer than about 200 characters (6.0.8), and macOS and Linux.

**VS Code check, 2026-10-23, on this machine (counts only).** After the real move of two repos on 2026-10-22, VS Code's `globalStorage/state.vscdb` still held 16 occurrences of their old paths, and `storage.json` held 2. `workspaceStorage` had folders for both the old paths (2) and the new ones (3). VS Code had started new, empty state for the moved folders. Its source (`src/vs/platform/workspaces/node/workspaces.ts`) computes a local folder's storage id as the MD5 of the folder's `fsPath` followed by its creation time: `birthtimeMs` rounded down on Windows, `birthtime` on macOS, and the inode on Linux. That's why REQ-21 re-keys workspace storage (21.0.2) and edits the SQLite recent list (21.0.1), rather than only `storage.json` and `workspace.json`, which is what `CLAUDE.md`'s notes covered.

**Found by reviewing the reference script:** it matches worktree project folders by encoded-name prefix (`<key>-*`). Since the encoding is lossy, that would also catch a sibling folder such as `website-old`. REQ-6 (6.0.2 to 6.0.4, 6.5) requires reading the real path from the transcripts instead.

## 4. Defaults proposed for the owner to confirm

These fill gaps in product.md. Each is written into the requirements above, and can be changed before approval:

1. **Editor state is VS Code stable only** (REQ-21). Other editions are reported, not handled. JetBrains is detect-and-report (REQ-11).
2. **Paths inside the workspace are never updated automatically** (REQ-10). Record-like files are unselected by default, and ignored files are skipped unless asked for.
3. **Backups are kept until the user prunes them**, never removed automatically (12.0.5). Undo after pruning reverses what it can and says what it can't (14.0.4).
4. **Any running Claude Code process blocks changes to Claude Code's data** (2.0.5), not just processes for this workspace, because any of them may write `~/.claude.json` back. This is safe but strict: with several sessions open, all of them must close, or the move waits with `--when-free`.
5. **A move finished later never applies opt-ins** (19.0.4), because the user may no longer be watching when it runs.
6. **`githubRepoPaths` always follows the move**, even with `--fresh-settings` (7.0.2), because it records where a clone lives, not a setting.
7. **`live_cwd` and `git_state.root` are never changed, even with `rewrite sessions`** (8.0.7), because they record where the session was.
8. **A folder holding several workspaces moves as one** (1.0.5), each listed in the plan, rather than being refused.
9. **`--yes` allows the default move without a prompt** (3.0.6), so an agent can run it after the user's go-ahead. It never turns on opt-ins.
10. **Performance limits** (REQ-28) are first guesses at "a typical move takes seconds".
