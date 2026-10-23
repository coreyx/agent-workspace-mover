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

## VS Code (TASK-3)

Not yet checked.

## Platform (TASK-4)

Not yet checked.
