# Folios

A folio is a report that an agent writes and Launchpad renders: a plan, an audit, a brief, a status board or a sketchbook. The agent writes Markdown with typed blocks. Launchpad shows it with one design on the laptop and the phone, keeps every rev, and lets you comment, decide and mark items on the page. **Send feedback** then gives your marks to the agent in one short message.

Use a folio when you want to review an agent's results and decide on them: a roadmap with items to accept or postpone, findings to fix, options to choose from, UI sketches to compare. A static website or a Space still works for a public, custom site; a folio is for work you review inside Launchpad.

Folios belong to a workspace. They are on the **Folios** page (in the sidebar, and in **More** on the phone), in each agent's **Folio** tab, and at `/folios/<slug>`.

## How it works

1. An agent runs `launchpad folio create --title "…" --kind roadmap`, then writes pages with `launchpad folio write`, `append` and `patch`. Home checks every write and prints the problems it found.
2. You open the folio from the agent's **Folio** tab, from the Folios page or from Today (**Open folio** on the agent's row).
3. You comment on text, set a verdict on items, pick options and tick boxes. Each mark is saved on Home at once and stays unsent.
4. **Send feedback** sends your unsent marks in one message to the agent that wrote the folio.
5. The agent reads them with `launchpad folio feedback`, changes the folio, replies to your comments and resolves them. The folio gets a new rev.

## The format

A folio is a folder of files that Home stores: `index.md`, more pages, images and data files (`assets/`, `data/`) and sketches (`sketches/`). Pages are [CommonMark](https://commonmark.org) with GitHub's extensions (tables, task lists, `> [!NOTE]` alerts), plus two additions:

- **`:::` containers** for layout.
- **Fenced YAML typed blocks**: a code fence whose language is a block name, with YAML inside.

`index.md` starts with front matter:

````markdown
---
title: Launchpad roadmap, winter 2026
kind: roadmap
status: for review
summary: Three waves to ship Folios, the store and reader first.
pills: [Feasible, ≈ 9 weeks]
pages: [index.md, plan.md]
---

:::callout{tone=verdict} Verdict
Ship the store and reader first. Comments come second.
:::

## Waves {#waves}

```roadmap
waves:
  - id: W1
    title: Store, CLI and reader
    size: M
    items:
      - { id: F-01, title: Plain store and records, advice: do }
      - { id: F-02, title: Session tab, deps: [F-01] }
```
````

- `kind` is one of `report`, `roadmap`, `audit`, `brief`, `status` and `sketchbook`. Each kind has a skeleton that `folio create` writes, with the usual reading order: verdict, decisions, details, plan, sources.
- `pages` lists the pages in reading order. `mood: editorial` uses serif headings.
- `{#id}` after a heading names its anchor. Marks hang on ids: headings, containers and typed items.

### Containers

| Container | What it shows |
| --- | --- |
| `callout{tone=…}` | An accent box. Tones: `verdict`, `info`, `note`, `tip`, `warn`, `risk`. GitHub alerts (`> [!NOTE]`) become callouts too. |
| `cards` | A grid of cards; each `###` heading inside starts a card. |
| `columns` | Columns side by side; each `###` heading starts a column. |
| `tabs` | Tabs; each `###` heading starts a tab. |
| `details` | A folded section; `{open}` starts it open. |
| `scope` | In scope and out of scope, with `### In` and `### Out`. |
| `hero` | A large lede under the title. |

Nest containers by adding colons to the outer one (`::::cards` around `:::callout`).

### Typed blocks

| Block | What it holds | Your marks |
| --- | --- | --- |
| `roadmap` | Waves of work, each with items (size, dependencies, the author's advice) | A verdict per item: **Do**, **Later**, **Won't do**, **Discuss** |
| `findings` | Audit findings with severity, place and fix | A verdict and comments per finding |
| `decision` | One question with options and the recommendation | Your choice and a note |
| `options` | Options side by side, with pros, cons and cost | Your choice |
| `decisions` | The decisions already taken: question, decision, effect | |
| `status` | Tiles with a state pill | |
| `timeline` | Dated events | |
| `steps` | Numbered steps | |
| `checklist` | Things to tick | Your ticks |
| `metrics` | Two to six key figures | |
| `table` | A sortable table from rows, or from a CSV file of the folio | |
| `chart` | A bar, line, area or pie chart from rows or a CSV file | |
| `prompt` | A prompt for an implementer, with **Copy** and **Launch with this prompt** | |
| `sketch` | A UI sketch in HTML, CSS and JavaScript (see [Sketches](#sketches)) | Comments |
| `sources` | References | |

A ```` ```mermaid ```` fence draws a Mermaid diagram (flowchart, sequence, state…). Any other fence is code.

Agents look up a block with `launchpad folio blocks <name>`, which prints its fields and an example. An unknown block such as ```` ```x-poll ```` still renders, as a key and value card. Home counts these unknown blocks and containers: the Folios page lists them under **Block requests**, as candidates for the catalog.

Images and data files come from the folio itself (`launchpad folio add-media`), never from the network: the reader shows a network image as a blocked placeholder.

### Sketches

A `sketch` block shows HTML, CSS and JavaScript with fake data in a frame: `src: sketches/<name>.html` or inline `html:`, and `frame: phone`, `laptop`, `panel` or `none`. Use sketches for UI ideas you want to compare.

- The frame gets Launchpad's theme variables and a small kit of classes: `lp-bar`, `lp-row`, `lp-card`, `lp-btn`, `lp-chip`, `lp-pill`, `lp-muted`, `lp-title`, `lp-panel`, `lp-stack`, `lp-grow`.
- Shared parts go in `sketches/_kit.html`, which every sketch of the folio loads.
- Keep each sketch under 15 KB, and all sketches of a folio under 200 KB.
- A sketch can keep state in the folio's [data](#the-data-api) with `folio.data.get`, `set`, `delete` and `list`, only under its own key prefix: its file name, else its block id, then `:` (or its `data:` field). For example, a poll in `sketches/poll.html` stores `poll:ranking`. On an older rev, a sketch can read but not write.

### Charts and Mermaid

The reader draws charts and diagrams itself, with no network. A chart takes `rows` (a label, then values) or `src:` a CSV file of the folio, with `type`, `x`, `y` and `unit`. A Mermaid diagram cannot show images or link outside the diagram.

## The check

Home checks every write and lists each problem with its file, line, rule and a hint. A write with errors is still saved, so the agent never loses work; only `--strict` refuses it. The reader shows problems in place, as a box at the block that has them, and in the **Problems** panel.

| Codes | What they check |
| --- | --- |
| F1 | Structure: front matter, unclosed containers and fences, unknown containers and attributes, pages, duplicate anchors, alerts, versions kept by a concurrent write (F108) |
| F2 | Typed blocks: YAML, required and unknown fields, values, duplicate ids, marks whose id is gone (F206), items without an id, unknown blocks, dependencies |
| F3 | Links, images and data files: missing pages, anchors and files, network images, table columns |
| F4 | Sketches: size, network use, data keys outside the prefix, unclosed `<script>` or `<style>` |
| F5 | Hints for the kind: a missing verdict, heading levels that jump |
| F6 | The phone: pages over 60 KB, tables with more than 7 columns, long code lines |
| F7 | What your reader saw: sketch errors and empty frames (F701), diagrams and charts that cannot draw (F702), a page that scrolls sideways on a phone (F703) |

F7 problems come from your own window: the reader reports them for the rev and page it showed, and `launchpad folio check` lists them as `render`. They do not change the exit code.

`launchpad folio check <slug>` exits with 1 when the folio has errors; `--strict` also counts warnings. `--fix` applies only the fixes with one clear answer (a misspelled field or container name) as a new rev. `--dir <folder>` checks local files offline. `launchpad folio rules <code>` explains a rule with a bad and a good example.

When an agent renames an item you marked, it adds `renamed_from: [<old id>]`, and your marks move to the new id. Otherwise F206 lists the marks whose id is gone.

## The Folios page

The page lists the workspace's folios, with the filters **Active**, **Shared** and **Archived** and a search field when there are many. Each row shows the title, the kind, who wrote it last, its rev, your unsent marks, its open comments, its errors and, when it is shared, how many days are left. **Open** shows it in the reader; **⋯** has **Archive** (or **Bring back**) and **Delete folio**. Deleting a folio keeps its topic and sessions.

A folio adds no Today row: unsent marks are drafts, not decisions, so their count stays grey.

## The reader

`/folios/<slug>` opens the reader full screen: on the laptop, the sidebar hides. The bar shows **Folios ▾** (a switcher with search), the title, the kind and rev, the agents button, **Ask**, **Launch**, **Comments**, **Send feedback** and **⋯**. The contents are on the left (the pages, then the headings of this page), and one side panel opens on the right: Comments, Versions, Problems or Share.

An agent's **Folio** tab shows the same reader inside the session, for the folios it wrote or worked on; **⤢** opens it full screen.

On the phone, the reader has ‹, the title and **Send**. The contents, the panels and Send feedback open as sheets. **Ask**, **Launch** and **Agents on this folio** are in **⋯**.

**⋯** also has **Versions**, **Share…**, **Problems** (when there are some), **Topic** (the folio's topic, with its sessions and traces), **Archive** and **Delete folio**.

The address keeps the page, the rev and the panel (`?page=&rev=&panel=`), so a link opens the same view.

## Marks and feedback

Your marks are:

- **Comments**: select text and click **Comment**, or use **Comment** next to a section, a container or an item. A comment keeps its quote; when the agent moves or edits the text, the reader finds the quote again, and shows the comment as outdated only when the quote is gone.
- **Verdicts** on roadmap items and findings: **Do**, **Later**, **Won't do**, **Discuss**. Click a verdict again to clear it.
- **Choices** on `decision` and `options` blocks.
- **Ticks** on checklists.

Each mark is saved on Home at once. **Send feedback · N** counts the marks you have not sent yet. Its menu offers, in this order:

1. The folio's thread: the bot thread that wrote the folio (or that got its last feedback).
2. The other agents that wrote it.
3. **New chat…**: a new chat with a bot you pick, with the folio's topic (its earlier agents) as context. That chat becomes the folio's chat.

You can add a line of your own. The message is short: it names the folio, counts your marks and gives the command that reads them (`launchpad folio feedback <slug> --since <cursor>`). Your marks become sent only once the message went, and only the marks it counted; a mark you edit meanwhile stays unsent. One Send runs at a time, so two windows never send the same marks twice.

The agent answers under your comments (`launchpad folio reply`) and closes them with the rev that addressed them (`launchpad folio resolve`). A sent verdict, choice or comment you remove later shows to the agent as `cleared`.

**Ask the author to fix**, in the Problems panel, sends one line to the same first recipient asking it to fix the folio's problems. Your marks stay unsent.

## Context and launching

A folio can be the context of a launch or a message, like a GitHub item or a trace:

- **+ Add context → Folio…** in the composer, an agent's message box, a bot's message box and the Ask panel: pick a folio, then the whole folio or one page.
- `@` suggestions: type `@` and the folio's name.
- In the reader, select text: the toolbar offers **Comment**, **Ask bot** and **Launch**. **Ask** and **Launch** in the bar do the same for the page.
- **Ask** sends your question to the folio's thread (or one of the recipients above), with the folio, page, section and quote as context. **Launch** opens the composer with the same context.
- A `prompt` block has **Launch with this prompt**: the composer opens with the prompt, the folio as context and the folder the block names, when one has that name.

The context is a reference, never the text. Home checks that the folio is in the session's workspace and freezes its rev. The agent gets one line with the folio's name and the `launchpad folio read` command for that part, plus your quote, fenced and labeled as text from the folio, not instructions. The agent joins the folio's topic.

The agents button (**N agents ▾**) lists the sessions that created or edited the folio, replied to your comments or got it as context, with their revs and status, and the folio's bot thread. Today's row of a finished agent that created or edited a folio has **Open folio** as its main button, after **Fix merge** and a Local PR. A bot's chat shows a receipt for each folio its turn wrote, and a session's trace lists them after the trace.

## The data API

Each folio has a small key and value store for sketches and agents, for example the result of a poll or a ranking you dragged in a sketch.

- Values are JSON. The last write wins.
- At most 64 KB per value, 1 MB and 2,000 keys per folio.
- Keys use letters, digits and `_ . : / -`.
- In your window, a sketch writes only its own prefix, at most 60 calls per 10 seconds, and only on the latest rev.
- Agents read with `launchpad folio data list <slug> --prefix <prefix>` and `data get`, and write with `data set` and `data rm`.

A shared snapshot has no data API: its sketches show their default state.

## Revs and versions

Every write is one rev, with its author and its problem counts. When an agent finishes a round of changes, it runs `launchpad folio done <slug> --message "…"`: its revs since its last done become one rev, numbered like the first of them and labeled with its message, so a folio keeps one rev per round instead of one per write. Done squashes only the agent's own newest revs: it stops at a rev of another writer or yours, at an earlier done, and at a named version (unless that version is the newest rev, whose name moves to the squashed rev). Comments, links and a shared snapshot that named a squashed rev name the new one. No content is lost, only the steps between. The **Versions** panel lists the revs, newest first: view a rev (the page shows **Back to the latest**; comments and data are read-only there), show its diff (of the source files), or **Restore before this rev**, which writes the files back as a new rev.

Two agents can write the same folio. A write names the rev it read (`--rev <n>`). When a file changed since, Home keeps both versions: the second one is saved as `<name>.concurrent-<id>.md`, and the check lists it as F108 until an agent merges them. A patch of a section that did not change since goes in place. A removal of a file changed since keeps the file.

The files of a rev are kept for 30 days. **Name this version** keeps a rev for good, under a name such as "v2 · sent to review"; agents name versions with `launchpad folio version`. **Unpin** makes it an ordinary rev again.

## Archive

A folio with no change, no mark activity and no open or unsent mark for 30 days archives itself. **Bring back** restores it as it was, with its files, revs and marks. A write to an archived folio also brings it back.

Home's backups include folios. After each backup, Home also writes a readable copy of each folio at its latest rev to the backup destination, under `<workspace>/folios/<slug>/`, next to the knowledge base and the bots. Home never reads these copies back.

## Share a snapshot

**⋯ → Share…** publishes a read-only snapshot of one rev as a static Hugging Face Space under your namespace, `<you>/folio-<slug>`, with Home's Hugging Face login.

- **Private** (the default) or **Public** (anyone with the link).
- **Delete after 30 days** (the default) or **Keep**. **Extend 30 days** and **Keep** change it later.
- **Show your decisions as text** adds your verdicts, choices and ticks. Comments, the data API and live updates are never in a snapshot.
- **Update snapshot** publishes the latest rev again; **Open the Space** opens it; **Stop sharing** deletes the Space at once.

Publishing is your click only, with a confirmation. An agent never publishes: when you ask, it prepares a ready card (`folio.share`) that waits for your click in Today. Home refuses to publish over a Space it did not create, so Stop sharing, Delete and expiry only delete Spaces Home created. Home deletes expired snapshots, and those of deleted folios, every hour. Deleting a folio stops sharing first.

A snapshot uses the folio viewer, a small build of the reader without Launchpad's API. Protected Spaces are not offered yet.

## Security

- Markdown renders with raw HTML off and is cleaned by DOMPurify. Links in typed blocks take only http(s) and mailto addresses.
- Images and files load only from the folio, through a sandboxed route, never from the network.
- Sketches are the only agent JavaScript. They run in a sandboxed frame with an opaque origin (`sandbox="allow-scripts"`, no same origin, forms, popups or top navigation) and a strict content security policy: no network at all, images and fonts only as data URLs. The reader inlines the folio's images for them.
- A sketch's only way out is the `folio.data` bridge, which the reader checks (the frame, a token, the key prefix, the rate and the size) before it calls Home.
- The app and the folio viewer set `frame-src 'none'`, so a sketch cannot navigate its frame to another address to send data out. A frame that loads a second time is stopped, shows “This sketch was stopped” and is reported as F701.
- Mermaid diagrams render with Mermaid's strict mode and no HTML labels, and the SVG is cleaned: no images, foreign objects or outside `url(…)`.
- A sketch can still leak what it sees through WebRTC, which the browser does not cover by its content security policy: its own data keys and what you type into it.

## For agents: the CLI

Agents use `launchpad folio …` with their session's credential; the [launchpad skill](../skills/launchpad/SKILL.md) explains the workflow.

| Command | Access | Purpose |
| --- | --- | --- |
| `folio list [--archived]` | read | The workspace's folios |
| `folio outline <slug>` | read | Pages, headings and typed blocks with their ids, problems and open comments |
| `folio read <slug> [page] [--section <id>] [--items <id>,…] [--rev <n>]` | read | Only the part the agent needs |
| `folio check <slug> [--fix] [--strict] [--json]`, `folio check --dir <folder>` | read (`--fix` propose) | The check |
| `folio feedback <slug> [--since <cursor>]` | read | Your sent marks, and `cleared` lines |
| `folio items <slug> [--type findings] [--verdict do] [--open]` | read | Typed items with your marks, as JSON |
| `folio revs <slug>` | read | The revs, newest first |
| `folio blocks [name]`, `folio rules [code]` | read | The catalog and the rules |
| `folio data list\|get <slug> [key] [--prefix <p>]` | read | The data API |
| `folio create --title … --kind … [--slug …]` | propose | A new folio from its kind's skeleton |
| `folio write\|append <slug> <path> [--file <f>] [--rev <n>] [--strict]` | propose | Write or append to a file (stdin by default) |
| `folio patch <slug> <path> --section <id> [--rev <n>]` | propose | Replace one section, container or typed block: the cheapest update |
| `folio add-media <slug> <file> [--as assets/<name>]`, `folio rm <slug> <path>` | propose | Add an image or data file, or remove a file |
| `folio reply <slug> <c-id> --text …`, `folio resolve <slug> <c-id> [--rev <n>]` | propose | Answer and close your comments |
| `folio data set\|rm <slug> <key> [json]` | propose | Change the data |
| `folio done <slug> [--message …]` | propose | Squash your revs since your last done into one rev |
| `folio version <slug> --name … [--rev <n>]` | propose | Name a version |

Every write prints `written rev N · 1 error, 2 warnings` and each problem. Hard limits refuse a write: a path outside the folio, a file over 2 MB, a page that is not UTF-8. Through a runner, one write may carry up to 8 MB.

Agents write as themselves: Home takes the author from its own record of the session, and the session joins the folio's topic. A bot's session also makes its thread the folio's thread, when the folio has none. Workspaces are a boundary: an agent reads and writes only the folios of its session's workspace. Your marks, Send feedback, Ask, Restore, Unpin, Share, Archive and Delete stay your clicks.

## A folio preset

Presets are yours (Settings → Presets). To ask agents for folios by default, add one, for example:

- Label: `Folio`
- Text: `Present your results as a folio: run launchpad folio list first and update the folio of the subject, or create one with launchpad folio create. Follow the Folios section of the launchpad skill, fix every error of launchpad folio check, and end with the folio's link and the decisions that wait for me.`

A preset for a static website still works for a public, custom site.

## Not built yet

- A **New folio** button: agents create folios.
- A diff with marks in the margin: the diff shows the source files.
- Screenshots of a folio taken by the agent (`folio check --shots`): F703 comes from your phone's reader instead.
- Search of folios from the top bar, and a folio mark in the Agents list.
