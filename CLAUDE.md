# agent-workspace-mover: notes for Claude Code

A command-line tool that renames or moves a workspace's folder (a git repo, a Perforce workspace, or any folder) and carries everything keyed on its path along with it: AI agents' sessions, memory and settings (Claude Code first), version control's links (git first), absolute paths inside the workspace, and editor state. It checks everything before changing anything, backs up what it edits, and can undo. Version 1 moves within one machine. A portable archive for moving between machines comes later.

**Read [spec/agent-workspace-mover/product.md](spec/agent-workspace-mover/product.md) first.** It says what the product is, who it's for, its goals, non-goals, core journeys and open questions. Don't restate it elsewhere; link to it.

## Where things stand (2026-10-23)

- **Nothing is built yet.** There's no code. The v1 spec in `spec/agent-workspace-mover/` is approved: `requirements.md`, `tech.md`, `design.md` and `tasks.md`. The stack is **Rust**, one binary per OS (see `tech.md`).
- **Decided by the owner:**
  - public and open source, under the MIT licence (`LICENSE`)
  - Claude Code only in v1, with each agent as a separate adapter
  - version control is adapters too: git in v1, Perforce planned next, and plain folders work with none
  - v1 also covers git internals, paths inside the workspace, editor state, and locks and running processes
  - **editing the record is always opt-in, never on by default.** A move may rename the folder that holds an agent's sessions. Settings keyed on the path (for Claude Code, `~/.claude.json`'s `projects` entry) **move by default**, because the owner wants them preserved, but the plan and report must say so, and a flag must let the user leave them behind and start fresh. It must not change a session's contents (such as `cwd` on each line), or history kept outside the sessions (such as `~/.claude/history.jsonl`), unless the user opts in for that run. No config file and no agent can switch that on for them, and each opt-in needs the user to **type a confirmation phrase** (such as `rewrite sessions`), never a Y/N. Runs with no one at a terminal can't opt in. See product.md, "Editing the record is always opt-in".
  - the name is agent-workspace-mover (renamed from git-repo-mover on 2026-10-22, so it isn't tied to git)
- **Not decided yet:** product.md's "Open questions for later", except those the spec settles. Section 4 of `requirements.md` records the defaults the owner approved.

## How work happens here

This repo uses the owner's SpecDevShip workflow. Its rules load from `~/.claude/rules/` every session: the workflow, spec conventions, coding standards, testing standards, and git conventions (Conventional Commits with `Spec:`/`Task:`/`Session:` trailers). Its skills are in `~/.claude/skills/`.

- **Next step:** `implement-tasks spec/agent-workspace-mover`, starting with Milestone 1. That milestone checks the design's "Facts to verify during implementation" (F-1 to F-10) before any code relies on them.
- **Gating:** no implementation until `tasks.md` says `status: approved`. A draft is not a go-ahead.
- **Testing:** full-tier coverage targets apply (see `~/.claude/rules/testing-standards.md`). This tool edits irreplaceable data, so treat the code that writes agent data, backups and undo as critical: 90–100%.
- **Commits** are credited to `coreyx`, with the GitHub no-reply address.

## What we know about the problem

The product came out of a real move. On 2026-10-22, two of the owner's repos were moved from an ad-hoc folder layout to a structured one. The script written for that move is the best reference implementation there is. It's kept outside this repo; `CLAUDE.local.md`, on the owner's machine, says where. It has a `-DryRun`, backups, and is safe to re-run. Read it before designing the Claude Code adapter. It follows the owner's rules: rewriting the `cwd` recorded in the sessions (`-RewriteSessionPaths`) and repointing prompt history (`-RewriteHistory`) are opt-in switches, off by default, and each needs its phrase typed (`rewrite sessions`, `rewrite history`) before it runs.

### Claude Code's path-keyed state

**This is observed, not documented.** It was seen with Claude Code 2.1.309 (native install) on Windows 11. Re-check every fact against the version you're testing with, and design adapters to detect versions and refuse what they don't recognise.

| Where | What's keyed on the path | Notes |
|---|---|---|
| `~/.claude/projects/<encoded path>/` | Everything about the project's sessions | **`<encoded path>`** is the folder's absolute path with every character that isn't a letter or digit turned into `-`. So `C:\Users\me\dev\github.com\me\website` becomes `C--Users-me-dev-github-com-me-website`. The drive letter appears in **either case**: `C--…` and `c--…` folders both exist on this machine, probably depending on how the folder was opened.<br><br>**Contents:**<ul><li>`<session-id>.jsonl` transcripts</li><li>`<session-id>/` folders, holding subagent transcripts (`subagents/*.jsonl`) and `tool-results/`</li><li>`memory/` (`MEMORY.md` plus one file per memory)</li></ul> |
| Each worktree inside the repo | Its own project folder | It's named after the worktree's path. For `.claude\worktrees\<name>` that's `<encoded repo path>--claude-worktrees-<name>`. |
| Every line of every transcript | `"cwd"` | Seen in **two forms in the same session**: a Windows path, JSON-escaped (`"C:\\Users\\…"`), and a POSIX-style one (`"/c/Users/…"`). **It isn't what resume matches on.** A test on 2026-10-22 renamed only a session's folder, leaving `cwd` at the old path. `claude -c`, `claude --resume <id>` and the `--resume` picker all found the session in the new folder (requirements.md, section 3). The test used edited copies, not a real interactive session, so confirming with one is still to do (design.md, F-1). **Rewriting it is an opt-in.** |
| First line of each transcript | `"entrypoint"` | `cli`, `claude-vscode`, `claude-desktop` or `sdk-cli`. **The picker hides `sdk-cli` (print-mode, `claude -p`) sessions**, which can be reached only by id. 84 of the 192 transcripts on this machine are `sdk-cli`. |
| User lines in transcripts | `serverClassifierContext.context.live_cwd` and `…git_state.root` | POSIX form. They look like a snapshot of where the session was when each prompt was sent, a record rather than a link. The 2026-10-22 move left them alone. They're session contents, so only an opt-in could change them, and arguably not even then, since they record where the session was. |
| `~/.claude/history.jsonl` | `"project"` on each prompt | This is prompt history, what the up-arrow recalls. **It was nearly missed in the real move.** **Repointing it is an opt-in:** by default, report the count and the opt-in. |
| `~/.claude.json` → `projects["C:/Users/…"]` | Per-project settings | Keyed with **forward slashes**. Holds `allowedTools` (permissions), `hasTrustDialogAccepted`, `mcpServers`, `enabledMcpjsonServers`, `lastSessionId` and the last session's stats. Worktrees have their own keys. |
| `~/.claude.json` → `githubRepoPaths["owner/repo"]` | A list of local clone paths | Windows form. |

**Not path-keyed:**
- `~/.claude/file-history/<session-id>/` holds snapshots of edited files, for rewinding. Transcripts index them by paths **relative** to the session's folder (`trackedFileBackups`), so they survive a move unchanged. Absolute old paths do appear inside the snapshots, but that's file content, which isn't the tool's to edit.
- `~/.claude/backups/` holds Claude Code's own backups of `.claude.json`.
- `~/.claude/sessions/<pid>.json` describes running processes and **contains tokens**. Never print it.

### Traps found the hard way

- **Windows won't rename a folder that any process is using as its working directory:** an editor, a terminal, a dev server, or the agent session that was asked to do the move. The tool must find what's holding a folder and say so, and handle "I'm running inside the folder I'm moving", usually by finishing after that session ends.
- **Running agents rewrite their config.** Any `claude` process (seven were running during the real move) can write `~/.claude.json` back and undo an edit made under it. Check for running agent processes before touching their data, and refuse rather than race them. A read-only dry run is fine while they run.
- **Git worktrees break in both directions** when the main repo moves. `git -C <repo> worktree repair <each worktree path>` fixes them. Submodules may hold absolute paths too (unverified).
- **Merge, never overwrite.** A session folder for the new path usually already exists: starting Claude Code in a folder at all, even with `--no-session-persistence`, creates its project folder with an empty `memory/`. Move entries across, and stop on any name that's already taken.
- **Match session folders by the paths recorded inside them,** not by their names. The encoding loses information, so the reference script's prefix match on names would also catch a sibling folder such as `website-old`.
- **Prove completeness by searching.** The real move found prompt history, VS Code's `storage.json` and the classifier fields by searching `~/.claude` and the editor's data for the old path, not by remembering where things live. It still missed VS Code's `state.vscdb` (see below). A post-move scan for leftovers is the tool's best guard against a store nobody knew about.
- **Edit JSON as JSON.** Windows PowerShell 5.1's JSON handling mangles large files like `~/.claude.json`. Regex is fine for the `cwd`-style rewrites in JSONL, but anchor it to the field name and the exact old path, and leave other paths and conversation text alone. A regex that edits what someone said is a bug.
- **Never print secrets.** Agent data holds tokens (`~/.claude/sessions/*.json`) and sometimes secrets inside conversations. Report counts and field names, not values.

### Editor state (VS Code, seen on Windows)

- **The recent-folders list is in SQLite,** not `storage.json`: it's the `history.recentlyOpenedPathsList` row of `%APPDATA%\Code\User\globalStorage\state.vscdb`. After the real move, that file still held 16 of the moved repos' old paths. The real move missed it.
- `globalStorage\storage.json` holds window state and backup workspaces, which also name the folder.
- **`workspaceStorage\<id>\` folders are keyed by an id computed from the folder,** not just mapped to it by `workspace.json`. VS Code's source (`src/vs/platform/workspaces/node/workspaces.ts`) computes the id as the MD5 of the folder's `fsPath` followed by its creation time: `birthtimeMs` rounded down on Windows, `birthtime` on macOS, and the inode on Linux. So editing `workspace.json` alone doesn't work, and the storage folder has to move to the new id. After the real move, `workspaceStorage` had folders for both the old paths (2) and the new ones (3): VS Code had started new, empty state for the moved repos. There were 65 such folders on this machine.
- Each workspace's own `state.vscdb` holds file URIs too, for example its open editors.

v1 handles all of these (requirements.md, REQ-21).

### Other path-keyed state to verify

v1 detects and reports these, and later versions fix them (product.md, Goals). **They come from general knowledge, not from testing on this machine,** so confirm each before building on it:

- **Python virtual environments** hard-code their absolute path (in `pyvenv.cfg`, the activate scripts and script shebangs). Pipenv and Poetry may keep a project's environment outside it, in a folder named after a hash of the project's path.
- **Docker Compose** takes a project's name from its folder unless `name:` or `COMPOSE_PROJECT_NAME` sets it. Containers, networks and named volumes carry that name, so a renamed folder starts a new, empty project.
- **direnv and mise** keep allow or trust lists keyed by the path of the `.envrc` or config file.
- **JetBrains IDEs** keep per-project caches and workspace state outside the project, keyed by its path.
