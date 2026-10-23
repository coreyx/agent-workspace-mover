# Facts verified during implementation

The results of checking the facts listed in [design.md](design.md), "Facts to verify during implementation" (F-1 to F-10). Each check ran on Windows 11 against throwaway folders, and its test data was removed afterwards. Paths here are generic: `<scratch>` stands for the throwaway folder's parent.

Where a result changes how a later task should be built, it says so. None of the results so far contradicts the design.

## Claude Code (TASK-2), 2026-10-23

Versions seen: the native CLI 2.1.309, the VS Code extension 2.1.308, and the desktop app's Claude Code 2.1.304. All three share `~/.claude`.

### F-1: a real interactive session resumes after a folder-only move. Confirmed.

- **Method:** the owner ran an interactive session (`entrypoint: cli`) in `<scratch>\f1a`, including one shell command, then exited. The folder was moved to `<scratch>\f1b`, and its project folder was renamed to the name for `f1b`, with no content change (the transcript's hash was unchanged). The owner then ran `claude --resume` from `f1b`.
- **Result:** the picker listed the session, and on resuming it the session recalled what had been said in it.
- **What the transcript recorded before the move:** 20 `cwd` values in the Windows form and 1 in the POSIX form (`/c/…`), and 1 `live_cwd` (POSIX form).
- **What resuming did:** appended 7 new lines to the same transcript, with `cwd` set to the new path. **After a move and a resume, one transcript records both the old and the new path.** design.md's matching rule ("Key design decisions", 7) already handles this, because it counts only the `cwd` values that encode to the folder's own name. TASK-12 needs a fixture for it.
- **Conclusion:** requirements.md 6.2 can be met by the default move. Rewriting `cwd` stays an opt-in.

### F-2: long project-folder names are shortened by a known rule. Found.

Claude Code names a project folder `encode(path)` (every character other than a letter or digit becomes `-`). When that's longer than 200 characters, the name is:

```
encode(path)[0..200] + "-" + base36(abs(hashCode(path)))
```

- `hashCode` is the Java-style string hash: `h = 31*h + code unit`, truncated to a signed 32-bit integer at every step, starting from 0.
- It's computed over the path as Claude Code saw it: the Windows form, with backslashes and the original case.
- `abs` is the absolute value of the signed result, not its unsigned reading. A path whose hash was negative confirmed this: the unsigned reading didn't match.
- Checked against five folders whose encoded names were 210, 210, 210, 230 and 250 characters long. Folders at 190 and 200 characters kept their full names.

**Consequence:** TASK-12 computes these names, rather than reporting the blocker that 6.0.8 allowed for when a name can't be derived with confidence. The drive-letter case used for the hash still needs care, since `C:` and `c:` give different hashes. A transcript's `cwd` values say which form was used.

### F-3: with `CLAUDE_CONFIG_DIR` set, everything moves into that folder. Found.

A session run with `CLAUDE_CONFIG_DIR` pointing at an empty folder created, inside it:
- `.claude.json`
- `backups/`
- `projects/<encoded path>/` (with a transcript and `memory/`)
- `sessions/`

Nothing was written to `~/.claude` or `~/.claude.json`. Login is kept per folder too: the session reported "Not logged in", but still wrote its transcript.

**Consequence:** `Locations` (TASK-7) puts the global config at `$CLAUDE_CONFIG_DIR/.claude.json` when the variable is set, and at `~/.claude.json` otherwise.

### F-4: worktrees have their own project folder, and memory belongs to the main repository. Found.

A session run in a worktree at `<repo>\.claude\worktrees\wt1`:
- recorded its transcript in `projects/<encoded repo>--claude-worktrees-wt1/`, the worktree path's own encoding
- recorded `cwd` as the worktree's path, and `gitBranch` as the worktree's branch
- also created `projects/<encoded repo>/memory/` (empty) for the main repository

**Consequence:** a move of the repository must carry both folders, which design.md's prefix matching does. Memory written from a worktree session appears to live in the main repository's folder. TASK-12's fixtures should include memory there.

### F-5: how Claude Code processes look. Found, except the npm install.

| Install | Executable |
|---|---|
| Native CLI | `~\.local\bin\claude.exe` |
| VS Code extension | `~\.vscode\extensions\anthropic.claude-code-<version>-win32-x64\resources\native-binary\claude.exe` |
| Desktop app (Store package `Claude`) | `<WindowsApps package>\app\claude.exe` for the app itself, and `%LOCALAPPDATA%\Packages\Claude_<publisher id>\LocalCache\Roaming\Claude\claude-code\<version>\<hash>\claude.exe` for its sessions |
| npm (`@anthropic-ai/claude-code`) | Not installed on this machine. Expected to be `node` running the package's CLI script. Unverified. |

- Every form is named `claude.exe`, so the process name can't tell them apart. **Any process named `claude.exe` counts as Claude Code for 2.0.5.** That includes the desktop app's own process, which hosts sessions and may write the same data. The executable's path says which install it is, for the report.
- Native CLI processes were started from `cmd.exe` or PowerShell, some with `--resume`.
- **Not path-keyed, newly seen:** `~/.claude/session-env/<session id>/`, an empty folder per session. Like `file-history/`, it's keyed by session, so it survives a move unchanged.

## VS Code (TASK-3), 2026-10-23

Version seen: VS Code 1.143.0 (stable, user install). All checks were read only, on copies of the databases, which were deleted afterwards.

### F-6: the workspace storage id formula. Confirmed on real data.

```
id = md5( fsPath + String(Math.floor(birthtimeMs)) )   (hex)
```

- `fsPath` is the form VS Code's `URI.fsPath` gives for the folder's `file://` URI. On Windows that's backslashes, with the **drive letter lowercase** as in the URI (`c:\…`).
- Of 64 `workspaceStorage` folders: 62 were single-folder workspaces and 2 were multi-root workspaces (`workspace.json` names a `.code-workspace` file rather than a folder).
- Of the 62, 20 named a folder that no longer exists. Those are orphans, for `scan` (REQ-18).
- **All 42 whose folder exists matched the formula**, none with an uppercase drive letter.

**Consequences:**
- TASK-19 uses the URI's own drive-letter case, and its self-check (21.0.5) should pass on a typical machine.
- Multi-root workspaces aren't covered by REQ-21. TASK-19 reports a `.code-workspace` file under the source as detected but not handled.

### F-7: the recent list and per-workspace state. Found, with one surprise.

**The recent list.** VS Code 1.143's own code still keeps it under `history.recentlyOpenedPathsList`, in the application storage (`globalStorage/state.vscdb`, table `ItemTable`), saved as JSON:

```
{"entries": [ {"folderUri": "<URI>", "label"?: "...", "remoteAuthority"?: "..."}
            | {"workspace": {"id": "...", "configPath": "<URI>"}, "label"?, "remoteAuthority"?}
            | {"fileUri": "<URI>", "label"?, "remoteAuthority"?} ]}
```

URIs are written by `URI.toString()`: `file:///c%3A/…`, with a lowercase drive letter and the colon encoded.

**The surprise:** on this machine the row is absent. No file in VS Code's data contains the key, so the list is currently empty and not stored. The 16 old paths the real move left in `state.vscdb` (requirements.md, section 3) weren't in a recent list. They were in the built-in GitHub extension's row, `vscode.github`, whose keys are `branchProtection:<folder URI>`: a per-repository cache of branch-protection rules, which the extension rebuilds when it's missing.

**Per-workspace state.** In one workspace's `state.vscdb` (86 rows), 10 rows held the folder's paths:
- 8 as plain JSON: for example the explorer's tree state, file history (`history.entries`), source-control repositories, and extensions' own state
- 1 as JSON nested inside a string: `memento/workbench.parts.editor`, the open editors
- 1 not JSON: `debug.selectedroot`

**Consequences:**
- The open editors can be rewritten as structured JSON (design.md, "Key design decisions", 13), so 21.1's "open editors" example stands, and nothing needs raising with the owner.
- `debug.selectedroot` holds a bare path or URI, not JSON. TASK-19 can rewrite it as a single path value, or else must report it as left alone (13).
- The `vscode.github` cache isn't something REQ-21 asks to carry. TASK-19 declares it an **expected leftover** (16.0.3), described as "the GitHub extension's branch-protection cache, which it rebuilds", so it doesn't show up as an unexpected warning after every move.
- TASK-19's fixtures need a recent list, built from the serialiser's shape above, since none was available on this machine to copy.

## Platform (TASK-4), 2026-10-23

The spikes are in `crates/awm-testkit/examples/platform_spikes.rs`. `.github/workflows/spikes.yml` runs them on GitHub's Windows and macOS runners.

### F-8: the Restart Manager is fast enough, but has a hard limit of 65,535 files per session. Found.

Time to register N files (one held open by the spike itself) and ask which processes hold them, registering all of them in one call:

| Files | This machine | CI Windows runner |
|---|---|---|
| 1,000 | 0.15 s | 0.07 s |
| 10,000 | 0.8 s | 0.35 s |
| 65,535 | 5.0 s | 3.4 s |

- It named the holding process correctly at every size up to 65,535.
- **At 65,536 files in one session, `RmGetList` returns error 29 and reports no holders at all.** That's a silent "nothing holds these files". **At 70,000 and 100,000, it crashed the calling process** with an access violation inside the Restart Manager.
- Registering in many small calls is slower than one large one (about 8.7 s against 5.0 s at 65,535), because the lookup, not the registration, dominates.

**Consequences for TASK-8:**
- **Never register more than 65,535 files in one Restart Manager session.** Split larger sets across sessions (50,000 each leaves a margin).
- **Treat any `RmGetList` error as "couldn't tell"** (`complete: false`, 20.0.4), never as "no holders".
- **Planning's 3-second budget** (design.md, "Key design decisions", 9) covers about 40,000 files on this machine, and more on faster hardware. A bigger workspace gets a partial check while planning, and the full diagnosis only if the rename fails, as designed.
- Since the crash takes down the caller, the platform layer should set the 65,535 limit as a constant with a test, not leave it to a caller's batch size.

### F-9: a waiter outlives its starter. Confirmed for Task Scheduler; the detached route is partly checked.

| Started from | In a job? | Task Scheduler (standard user) | Detached, breakaway requested |
|---|---|---|---|
| Claude Code's shell tool | no | registered, started, **finished after the shell command ended** | breakaway allowed; started, **finished after the shell command ended** |
| Windows Terminal (PowerShell) | no | registered, started, finished (in its own window; see below) | breakaway allowed; started, finished, **but the terminal was still open, so closing it wasn't tested** |
| GitHub's Windows runner | **yes: breakaway not allowed**, no kill-on-close | not tried | would fall back to no breakaway |

- A task's process isn't a child of whoever registered it, so closing the starting terminal can't affect it. It ran to the end from both starters.
- **Whether a detached process survives its Windows Terminal window closing is still unverified.** TASK-28's test for 19.2, which closes the starting terminal on each OS's runner, has to cover it.
- **A task that runs as the logged-in user (the interactive-token logon type) opens a visible console window** for a console program, and keeps it open while the program runs. The owner saw it appear, and close on its own when the 60-second payload finished.

- Registering a one-off task as a standard user works: a registration trigger, the interactive-token logon type, and no administrator rights.
- **Breakaway isn't always available**: the CI runner's job forbids it. So the Task Scheduler route stays primary and the detached process is the fallback, as tech.md planned. TASK-28 must check the job's limits (as the spike's `job` subcommand does) before relying on the fallback.
- **TASK-28 must start the waiter without a window**, since a waiter can wait up to 24 hours. The options are:
  - a small helper binary built for the Windows subsystem, so it has no console
  - the waiter freeing its console at once, which still flashes a window briefly
  - a logon type that doesn't run on the user's desktop

  The choice changes what ships, so it's raised with the owner in TASK-28.

### F-10: macOS process inspection needs no extra rights. Confirmed on GitHub's macOS runner.

- `proc_pidinfo(PROC_PIDVNODEPATHINFO)` read another process's working folder, and it matched.
- `proc_pidinfo(PROC_PIDLISTFDS)` plus `proc_pidfdinfo(PROC_PIDFDVNODEPATHINFO)` listed its open files and found the one it held.
- `pathconf(_PC_CASE_SENSITIVE)` reported the default volume as case-insensitive (0).
- libc 0.2 lacks `vnode_fdinfowithpath` and `PROC_PIDFDVNODEPATHINFO`. The spike declares them with the SDK's layout, and so will TASK-8.
- The `lsof` fallback wasn't needed. It stays in the design for processes `libproc` can't inspect.
