# Product: agent-workspace-mover

## Problem and motivation

Renaming a workspace's folder (a git repo, a Perforce workspace, or any folder you work in), or moving it somewhere else on disk, looks like the most ordinary file operation there is. For anyone who works with AI coding agents, it quietly destroys things.

Agents and developer tools key their state on a folder's absolute path. Move the folder and that state is orphaned. It isn't deleted, but nothing finds it any more:

- **Agent sessions disappear.** Claude Code files a project's conversations under a folder named after its path, and records the path again inside every session. After a move, `claude --resume` in the new folder lists nothing. Weeks of context, decisions and subagent work are still on disk, but out of reach.
- **Agent memory and settings reset.** Saved memories, granted permissions, trusted-folder status and MCP server choices are keyed by the old path. The agent starts again as a stranger: re-asking for permissions and forgetting what it had learned.
- **Prompt history thins out.** The prompts recalled with the up-arrow are tagged with the folder they were typed in.
- **Version control loses track.** Git worktrees record absolute paths in both directions, so moving the main repo breaks them. A Perforce workspace's client spec names its root folder on the server, so after a move Perforce looks for the files in the old place.
- **Absolute paths inside the repo go stale.** Configs, scripts and docs that name the old location point nowhere.
- **Editors lose their place.** Recent-folder lists, workspace storage and per-folder settings refer to a path that no longer exists.
- **The move itself often fails, or half-fails.** On Windows, a folder can't be renamed while any process is using it: an editor, a terminal, a dev server, or the agent session that was asked to do the move. Running agents also rewrite their config files, and can overwrite a careful fix made while they run.

Today the only mitigation is doing all of this by hand, every time. That means discovering where each tool keeps its state, writing a one-off script, and hoping nothing was missed. The owner hit this often enough to write that script for a real move: it moved two repos, two session folders (8,800 recorded paths), 152 prompt-history entries, project settings and a git worktree. Even then, prompt history was nearly missed. That script is the seed of this product.

## Target users

- **Developers who use AI coding agents day to day** and keep long-lived sessions per repo. For them, session history and agent memory are valuable working context, not disposable logs. This is the core user, starting with Claude Code users.
- **People reorganising their folders:**
  - moving from an ad-hoc layout to a structured one (for example `~/dev/github.com/<user>/` to `~/repos/mine/github.com/<org>/`)
  - renaming a repo after a product rename
  - re-homing a repo after it moves to a new GitHub organisation
- **Later, people changing machines,** who want their agent history to come with them (see Goals, "Beyond v1").

It's a command-line tool for people comfortable in a terminal. It's public and open source, so it must cope with setups other than the owner's:
- Windows, macOS and Linux
- different install methods and versions of each agent
- repos with worktrees and submodules
- several agents in use on one machine

## Value proposition

- **One command instead of an afternoon.** `agent-workspace-mover move <from> <to>` does the whole job: the folder, the agent data, version control's links, and the paths that point at it.
- **It finds what you'd miss.** It knows where each supported tool keeps path-keyed state, so a move doesn't silently strand a worktree's sessions or a permissions list. What it won't change by default, like prompt history, it reports and offers as an opt-in, rather than leaving it unmentioned.
- **It's safe to trust with irreplaceable data:**
  - It checks everything before changing anything.
  - It shows exactly what it will do, with counts.
  - It backs up whatever it edits, and can undo a move.
  - It leaves the record alone unless you ask. By default it renames the folders that hold an agent's sessions, but never edits a session itself, or history kept outside the sessions, like prompt history. Those are separate opt-ins. Even then, it changes only the paths, never what was said.
- **It fixes moves that already happened.** If a repo was moved by hand and its sessions were orphaned, it can find them and reattach them.
- **It tells you what it couldn't do,** and why, rather than reporting success it hasn't earned.

Doing nothing means losing context or rebuilding it by hand. A hand-written script works once, for one machine, one agent version and one move.

## Goals and success criteria

**Version 1: moves on the same machine, with Claude Code.**

- **A move is complete.** After moving a repo, in the new folder:
  - `claude --resume` lists every session the old folder had, worktrees' sessions included
  - memories, permissions and trust are as they were (unless the user chose to start fresh; see below)
  - version control is healthy (for git, `git worktree list`)
  - any absolute paths inside the repo were found and offered for update
  - everything left unchanged by default (paths recorded inside sessions, prompt history) is reported, with the opt-in that would update it
- **Settings move by default, visibly, and can be left behind.** An agent's per-project settings keyed on the path carry across by default, because losing them is rarely what anyone wants. For Claude Code, that's `~/.claude.json`'s permissions, trust and MCP servers. The plan and the report always call this out, listing what moves. A flag (`--fresh-settings`, say) leaves them where they are, so the agent starts fresh in the new folder. The old settings stay under the old path, untouched, and the scan can find them later.
- **Editing the record is always opt-in.** By default, a session's folder may be renamed, and settings may move as above. Two things are never changed unless the user asks for them, by name, for that run:
  - **a session's contents,** such as the working folder recorded on each line
  - **history kept outside the sessions,** such as Claude Code's prompt history (`~/.claude/history.jsonl`)

  The plan always lists what an opt-in would change, with counts, so the choice is informed. No configuration, and no agent acting for the user, can turn an opt-in on by default.

  **Opting in takes a typed confirmation, not a Y/N.** After seeing exactly what it will change, the user types a short phrase that names the opt-in, such as `rewrite sessions`. A Y/N prompt is too easy to answer by accident. Anything other than the exact phrase skips the opt-in and leaves the record as it was. A run with no one at a terminal to type it (a script, or an agent driving the tool) can't opt in at all. It reports the command the user can run themselves.
- **Nothing is lost, ever:**
  - every file it changes is backed up first
  - a failed or interrupted move can be resumed or rolled back
  - `undo` restores the previous state
- **Problems are caught before anything changes:**
  - a target that already exists
  - folders or files in use, and by what
  - agents still running
  - session data from an agent version it doesn't recognise
  - name collisions between the old and new session folders

  Each is reported with what to do about it.
- **The dry run is exact.** Running with `--dry-run` lists every action and count that the real run would take.
- **Orphans can be rescued.** A move done by hand earlier can be repaired after the fact. A scan can list agent data whose folder no longer exists.
- **Workspaces are recognised by identity, not only by path.** For git, that's the first commit and the remotes; for other version control, its own equivalent (a Perforce client name, say). So rescue and the scan can match orphaned agent data to wherever the workspace lives now, without being told the old path.
- **A blocked move can finish itself.** When something is holding the folder (an editor, a terminal, or the agent session that asked for the move), the user can have the move recorded and completed once those processes exit, instead of having to come back to it.
- **Machine-readable output.** `--json` plans and reports, with stable exit codes, so the agent skill, other agents and scripts can drive the tool reliably.
- **Other things that break are detected and reported,** even where v1 doesn't fix them yet:
  - **Python virtual environments** inside the workspace, which hard-code absolute paths
  - **Docker Compose projects,** which take their name from the folder by default, so a rename strands their containers, networks and volumes

  The plan names each one and what the user can do about it.
- **Adding an agent or a version-control system is a contained job.** Every agent's knowledge (where it keeps data, what it records, how to recognise its versions) lives in one adapter, and so does every version-control system's. A second of either can be added without changing the core. Git is the first version-control adapter. A plain folder with no version control moves too.

- **Agents know how to use it.** It ships an agent skill (a `SKILL.md` in the open Agent Skills format, starting with Claude Code). When a user asks their agent to rename or move a workspace, the agent should:
  - use the tool rather than `mv`
  - show the plan and get the user's go-ahead
  - never turn on an opt-in (editing sessions or history) unless the user has asked for it in so many words
  - recognise that its own session is one of the things holding the folder
  - finish cleanly: hand the user the one command to run once the session has ended, or set the move to complete then

  The skill also tells an agent how to spot orphaned sessions from an earlier move done by hand, and offer to rescue them.

Success, measured:
- In real use, no move loses a session, a memory or a setting.
- A typical move takes seconds, plus the time to close what's using the folder.
- Users stop writing their own scripts for this.

**Beyond v1:**

- **More agents,** one adapter at a time. Likely candidates include Codex CLI, Gemini CLI, Cursor and GitHub Copilot.
- **More version-control systems,** the same way. Perforce is the first planned: updating the workspace's root in its client spec, on the server, as part of the move. Mercurial, Subversion and Plastic SCM are possible later.
- **A start-up hook for agents.** For example, a Claude Code session-start hook that notices the current folder has orphaned sessions elsewhere and offers to rescue them. It catches moves made outside the tool: in Explorer or Finder, or by re-cloning.
- **Batch moves and re-layout.** Move many workspaces in one run, from a mapping file or by a convention such as `~/repos/<host>/<owner>/<repo>` worked out from each workspace's remote.
- **Leave a link behind.** Optionally put a junction or symlink at the old path for a transition period, so shortcuts and scripts that still point there keep working, and remove it later.
- **Fix, not just report,** more of what breaks. That means re-granting path-keyed trust and allow lists (direnv's `allow`, mise's trust, editor workspace trust), carrying JetBrains IDEs' per-project state, and repairing the virtual environments and Compose projects that v1 only reports.
- **A shared adapter library.** The knowledge of where each agent keeps its sessions, and how to read them, packaged as a library that other session tools can reuse.
- **Moving between machines.** A portable archive would hold a repo's agent data, and optionally the repo itself. It would describe paths in a machine-neutral way, so it can be unpacked on another machine, under another user, folder layout or OS, and remapped to wherever the repo lands. The same format could serve as an off-machine backup of agent history, and for handing a session to someone else, with secrets redacted.

## Non-goals

- **Not a sync service.** It moves state once, on request. It doesn't keep sessions in step across machines or folders.
- **No editing of the record by default.** Session contents and history kept outside the sessions change only when the user opts in (see Goals). Even then, only recorded paths change, never what was said. Old paths in conversation text stay as they were, because they're part of the record.
- **Not a general backup tool.** It backs up what it changes, for undo and recovery, not a whole home directory. The cross-machine archive comes later and is scoped to agent data.
- **Not a hosting migration.** Renaming a GitHub repo, moving it between organisations, changing remotes, or moving Perforce depots and streams is the server's job. At most, agent-workspace-mover reports remotes that might need attention.
- **Not cross-machine in v1.** Same machine only, including across drives.
- **No agents beyond Claude Code in v1,** though the design must not assume Claude Code is the only one.
- **Not a cleaner.** It may report leftover agent data from deleted or temporary folders, but deleting anything is the user's decision, made explicitly.

## Core user journeys

1. **Rename or move a workspace.** The user runs `agent-workspace-mover move <from> <to>`.
   1. It checks everything first: locks, running agents, targets, data versions.
   2. It shows the plan and asks to go ahead.
   3. It moves the folder, updates version control, carries the agent data across and updates paths.
   4. It verifies the result, for example that the agent would list the sessions in the new place.
   5. It reports what it did, and anything it couldn't do.
2. **See before doing.** With `--dry-run`, or a `plan` command, the user gets the complete plan with counts and changes nothing. This is useful while agents are still running, to know what to close.
3. **Rescue a move that already happened.** The repo was moved by hand and the sessions are orphaned. The user points the tool at the old and new paths, or lets it find candidates. It reattaches the agent data to the folder as it is now.
4. **Undo.** The user runs `undo` and the last move, or a chosen one, is reversed from its record and backups.
5. **Ask your agent.** The user tells their agent "rename this repo to X" or "move it under ~/repos/acme". The skill guides the agent to plan the move with the tool, explain it, and finish it safely despite running inside the folder.
6. **Check the machine** (a doctor or scan). It lists agent data whose folder no longer exists, likely matches for it, and leftovers from temporary folders, so the user can rescue or deliberately clean them.
7. **Later: move to another machine.** The user exports a repo's agent data to an archive, carries it over, and imports it against the repo's new location there.

## Market and competitive context

Thin, and that's the opportunity. As of October 2026, the closest things are:

- **Version control's own tools.** `git worktree move` and `git worktree repair` fix git's links, and Perforce lets you edit a workspace's root, but neither knows about agents or editors.
- **Agents' own commands.** Claude Code has no command for moving a project's sessions or settings, and other agents seem to be in the same position. This should be re-checked before each release, and whatever an agent adds should be embraced rather than duplicated.
- **Advice and one-off scripts** in issues, forums and posts. They target one agent and one platform, have no dry run or undo, and go stale when the agent changes its storage.

The difference is being complete, safe and maintained across agents, versions and platforms, rather than a script that worked once.

## Constraints and context

- **Open source and public, under the MIT licence.** Every claim must hold on other people's machines. It can't assume the owner's folder layout, agent versions or OS.
- **Windows is first-class, not an afterthought.** It's the owner's platform and the hardest one: folders in use can't be moved, there are two path forms, drive letters can be either case, and paths are escaped inside JSON.
- **Agents' storage formats are undocumented and change between versions.** The tool must recognise what it understands, refuse or warn on what it doesn't, and never guess with someone's data. Each adapter must say which agent versions it has been verified against.
- **The data is personal and sensitive.** Sessions hold code, conversations and sometimes secrets. The tool makes no network requests, collects no telemetry, and keeps backups local. The future archive format has to treat this seriously.
- **It must not depend on the agent it's moving data for.** A user may ask an agent to do the move, so the tool has to detect the session it's running in and explain what to do. The usual answer is to finish the move after that session ends.

## Open questions for later

- **The command's name.** `agent-workspace-mover` is long to type. Should it also install a short alias, and maybe a `git-` one so `git workspace-mover` works?
- **Perforce's scope.** Changing a workspace's root needs a connection to the server and the right to edit the client spec. How does the tool handle being offline or lacking permission? And which other Perforce state on the machine is path-keyed?
- **How far "editor state" goes.** VS Code's recent folders and workspace storage are the obvious start. Which editors and IDEs, and how much of their state, is v1?
- **Paths inside the repo.** Offer every match for review, or update some kinds of file automatically? And how should it treat matches in history-like files (changelogs, logs) that are records rather than live references?
- **How long backups are kept,** and how `undo` behaves once they're cleaned up.
- **The archive format.** What's in it: agent data only, or the repo as well? How are paths described neutrally? How is sensitive content protected? Is it the same format as the backups?
- **Does a session still resume when only its folder is renamed?** If an agent checks the working folder recorded inside each session, its sessions may need the opt-in rewrite to be listed in the new place. That has to be tested for each agent and version. When the default can't make sessions resumable, the plan must say so plainly and name the opt-in, not leave the user to find out.
- **Working with the agents.** If an agent adds its own move or export command, should agent-workspace-mover call it, wrap it, or step aside?
