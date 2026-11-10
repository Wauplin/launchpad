# Knowledge

Each workspace has a knowledge base: plain Markdown notes and media files, the memory your agents share. Bots also keep a memory of their own (see [Bots](bots.md#memory)). Home stores it. Every change records who made it, and every change can be restored for 30 days.

## What agents see

- Every session on a runner gets a read-only copy of its workspace's knowledge base. The variable `$LAUNCHPAD_KB` points to it. Agents read it as files, for example with `rg`, or with `launchpad kb search <words>`.
- The harness's own memory is off in every session (Claude Code auto memory, Codex memories). Their user instructions point to the knowledge base and include `INDEX.md`, the map of the knowledge base, at the start of the session.
- Agents write with the `launchpad kb` commands: `write <path>` (stdin or `--file`), `append <path>`, `add-media <file>` and `rm <path>`. Home applies writes one at a time. The copy on the runner updates right after a write and every minute.
- Agents are told to write rarely: a few durable, high-level findings (your preferences and decisions, facts that took effort to discover, how-tos) and what you ask them to remember. They do not log sessions, runs, task status or results: traces and Git history keep that, and a log would fill the context of every session that reads the knowledge base.
- Repository instructions (`CLAUDE.md`, `AGENTS.md` in a repository) are not memory and stay where they are.

A new knowledge base starts with `INDEX.md` and `AGENTS.md`. `AGENTS.md` explains what to write and what not to write, the layout (`people/`, `projects/`, `topics/`, `decisions/`, `howto/`, `sources/`, `media/`) and the front matter of a note (`title`, `type`, `status: current | superseded`, `valid_from`, `sources`). Edit it to change how agents write.

## The Knowledge page

Open **Knowledge** in the sidebar. It shows the knowledge base of the window's workspace.

- **Notes** lists every file. Find a note by its name or its text. A note shows its front matter as labels and renders `[[wikilinks]]` as links. Below the note, you see who wrote it and the notes that link to it. **Edit** changes a note, **History** lists its changes and **Delete** moves it to the trash. **New note** creates one.
- **Bot memory**, at the end of the list, shows each bot of the workspace with its notes. A click on a bot opens its Memory panel. A note opens read-only, with **Open <bot>**: the bot writes its memory, and its versions are on its page.
- **Changes** lists every change of the last 30 days. Each change shows the agent (with its model and runner) or you, the lines added and removed, and the diff. Use **Open the session** to read the trace of the agent that made the change. **Restore previous** puts back the version from before a change. **Undo** removes a note that a change created, and **Restore** puts back a removed note. A restore is itself a change, so you can undo it too. Changes made since your last look have a dot. **Mark all seen** clears them, and the sidebar shows their count.

Launchpad does not judge whether a note is true. Review the Changes list when you want to.

## Concurrent edits

A write records the version of the note that the agent started from. If the note changed in the meantime, Home keeps both versions: the new one is saved as `<note>.concurrent-<id>.md`. The Notes list and the note show it. Click **Keep this note** or **Use the other version**, or let the gardener merge them.

## Your edits on a runner

On a runner, the copy is in `<runner data folder>/knowledge/<workspace name>/`, for example `~/.local/share/launchpad-runner/knowledge/Work/`. You can open that folder as a vault in Obsidian. Your edits there are sent to Home within a minute and are recorded as yours.

## The gardener

**Run the gardener** opens the composer with a prompt for the workspace: merge duplicate notes, cut notes down to their durable findings (removing logs of runs and results), add missing links, mark outdated notes as superseded, merge concurrent edits and keep `INDEX.md` current. Pick a profile with a cheap model. To run it every week, choose **Repeat on a schedule**. Its changes are listed and restorable like any other.

## Import memories

**Import memories** reads the memory files of Claude Code (`~/.claude/projects/*/memory/`) and Codex (`~/.codex/memories/`) on a runner you choose, then lists them. The import copies the selected files into `imported/<harness>/<project>/` as your notes. Nothing is removed from the machine. Then run the gardener to fold the files into your notes.

## Storage

Home stores the knowledge base like a plain folder: the file list is in its database, and the contents are stored by hash. Both go into every Home backup. After each backup, Home also writes a readable copy of the notes to the bucket, under `<workspace>/kb/`, and removes the files you deleted. Home never reads that copy back: edit notes on the Knowledge page or in a runner's copy.

Bots keep their own notes in their own memory, not in the knowledge base (see [Bots](bots.md#memory)). Home moved the notes that bots had kept in `bots/<bot>/` there once, at its first start after the update.
