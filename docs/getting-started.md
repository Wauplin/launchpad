# Getting started

This guide sets up a Home, adds a place where agents run, takes you through the first run and launches a first agent.

Launchpad has two parts:

- **Home** keeps every record, credential and branch, and serves the app to your browsers and phone. It runs no agents and no terminals: terminals you open in its window run on a runner.
- **Runners** run the agents: your Linux machines, a Local VM, or **Burst** jobs that a Home in a Hugging Face Space starts on its own. See [Home and runners](home-and-runners.md).

## Set up Home

### In a Hugging Face Space (recommended)

A private Space needs nothing on your machines. Run the one-line install from the [README](../README.md#try-it-yourself) with the hf CLI. Then open the Space and follow its setup page: Hugging Face sign-in, the credential key, GitHub, the Git identity of agents, an agent login, a workspace and a first folder. See [deploy/hf-home](../deploy/hf-home/README.md) for every step. Agents then run on Burst until you add a runner.

### Self-hosted on Linux

You need Rust 1.93 or newer and Node 24 with pnpm (through Corepack).

```sh
git clone https://github.com/Wauplin/launchpad.git
cd launchpad
packaging/install.sh
```

The script installs `~/.local/bin/launchpad` and the systemd user unit `launchpad.service`, which runs Home (`launchpad daemon`). Run it again to update; it asks first when agents are in a turn. With a prebuilt package (`launchpad-linux-x86_64.tar.gz` from a [GitHub release](https://github.com/Wauplin/launchpad/releases)), extract it and run `bash packaging/install.sh --prebuilt`. On a server, add `--headless` (see [Remote access](remote-access.md#run-home-on-your-own-server)).

Sign Home in to GitHub with `gh auth login` on the same machine. Then run `launchpad url` there: it prints a link that signs your browser in. Install the page as an app from the browser menu if you like.

A self-hosted Home has no Burst: add at least one runner.

## Add a runner

A runner is a Linux machine (Linux 5.9 or newer) that runs agents in a bubblewrap jail. It needs Git, bubblewrap and the CLI of each agent you use (Claude Code, Codex or OpenCode). It never uses the machine's own sign-ins: agents get Home's logins through a proxy.

1. Open **Settings → Home and runners → Add a runner** on Home.
2. With a Home in a Space: sign in to Hugging Face on the machine (`hf auth login`), then copy the install command and run it there. It checks the machine, installs Home's own build and starts the runner as a user service. With a self-hosted Home: start `launchpad daemon --role runner` on the machine (Launchpad at Home's commit), then run the command shown, `launchpad connect <Home address>`.
3. The machine prints a code. Check that Home shows the same code and click **Approve**.

On a Mac, create a Linux VM runner with `packaging/local-vm.sh <Home address>` (Multipass). Home refuses runners on other systems.

A runner's own window (its address on that machine) shows only its status: the link to Home, **Takes new sessions**, an update notice when its build differs from Home's, and its checklist. The checklist checks the link to Home, the jail, the network capture, the agent CLIs, the agent logins in Home's vault and the Git identity of agents. A missing part shows a command: copy it, run it in a terminal on that machine, then click **Check again**. Launchpad never runs these commands itself. Launch, review and change settings on Home.

## First run

Open Home in your browser.

### The tour

On a fresh install, a welcome dialog offers:

- **Essentials tour** (about 3 minutes): launch an agent, answer it and review its work.
- **Full tour** (about 10 minutes): adds GitHub, Ask Launchpad and bots with their routines, terminals, workspaces, automation, settings, and Home with its runners and credentials.
- **Skip the tour**: go straight to **Finish setup**.

![A tour step on demo data](images/tour.webp)

The tour runs in demo mode: a fictional organization with prerecorded agent runs. Nothing reaches Home, GitHub or your folders. A purple line at the top shows that you are in the demo, and **Exit demo** returns to the real app. You can replay any chapter later from **Settings → Onboarding**. The guided tours run on a computer; on a phone, **Settings → Onboarding → Explore the demo** opens the same demo without the tour.

### Finish setup

**Finish setup** in the sidebar (route `/setup`) lists Home's setup steps, what was found and where to finish each one. The sidebar counts the required steps left. On a self-hosted Home, the steps are:

- **Workspace**: at least one, with the name you choose. Launchpad creates none by itself. Adding or cloning a folder waits until a workspace exists.
- **GitHub**: Home's GitHub login (`gh auth login` on Home). The Inbox, issues, pull requests, Clone from GitHub and publication use it.
- **Agent login**: a profile and its login in Home's vault (**Settings → Credentials and tools**). Agents never use a machine's own sign-in.
- **Git identity of agents**: the name and email of your agents' commits.
- **Where agents run**: a paired runner, or Burst on a Home in a Space.
- **First folder**: clone a repository from GitHub or create a plain folder.

A Home in a Space reads the same list from its setup page, with its Hugging Face and key steps first. **Open** next to a step opens the page that changes it.

### Create a profile

A profile says how an agent runs: the harness, what pays for the work and the default model. Launchpad ships no profiles. Open **Settings → Profiles → New profile**, choose how the work is paid (a subscription, an API key or another provider), then pick a template. For a subscription, add its login in **Settings → Credentials and tools**: a Claude Code token (`claude setup-token`) or a Codex `auth.json`.

![Settings → Profiles](images/profiles.webp)

See [Profiles](agents.md#profiles) for the details.

### Add a folder

Launchpad starts with no workspace: create one in the **Workspace** step of **Finish setup**, or in the **Add folder** dialog, which offers it when there is none. The menu at the top of the sidebar lists your workspaces (see [Workspaces](agents.md#workspaces)). Open **Folders** in the sidebar and click **Add folder**. It has two tabs:

- **Clone from GitHub**: Home keeps a mirror of the repository, and runners clone it on first use. Suggestions list repositories with recent GitHub notifications first, then your recently pushed repositories.
- **New plain folder**: a folder without Git, whose files Home stores.

Every folder is stored by Home, so agents run in it on any runner. The sidebar lists **Bots** first (its chevron collapses the group on this device), then Today, Agents, Inbox, Schedules, Knowledge and Folders. The sidebar button next to Settings collapses the sidebar to icons on this device (counts become badges on the icons, and names show on hover); click it again to expand it. The Folders page lists the folders of the workspace and its temporary folders.

### Launch your first agent

1. Click **Launch** next to the field in the top bar, with the field empty: the composer opens. (Typing the task in the field first and clicking **Launch** starts at once with the page as context and the folder's defaults.)
2. Write the task. Type `@` to mention files, `/` for skills and `#` for folders, agent traces and GitHub items.
3. Choose the folder. The options line under the prompt says where the agent works, for example "New worktree from main · keep for review": in Git folders, a new worktree (the default) gives the agent its own branch. Click the line to change **When done** (Read-only, Keep for review, Local PR, Draft PR, Open PR or Merge), **From** (a new worktree from a branch, or an existing worktree or Local PR) or **Into** (the branch the work goes into, by default the one it starts from). The line "Runs on" names the runner: the one you chose, or by default a runner, else Burst. To only ask a question or request an audit, choose **When done → Read-only**: the agent changes nothing, and archiving it deletes its worktree.
4. Click **Launch**.

![The composer with its options line](images/composer.webp)

The agent appears in **Agents → Running**. When it finishes, a card waits in **Today** until you have looked at it or merged its branch. See [Agents](agents.md) for everything that happens next.

### Find your way

- **One action per row.** Lists and cards show one button per row or card: **Done** on Inbox rows, **Launch** on Folders rows, the main action on Today rows (**Merge**, **Review**, **Resume**, **Start**), **Start** on proposal cards. The other actions are in **⋯**: on Folders, **Files**, **Folder settings**, **Move to …** and **Remove**; on Agents rows, **Stop** or **Archive**, **Mark unread** and **Pin**; on proposal cards, **Dismiss** and **Edit**. Settings lists (profiles, tools, MCP servers, hooks, runners, presets) work the same way. In lists, **⋯** shows when you hover the row or focus it with the keyboard; on touch screens it is always visible. On the phone, Today rows have no **⋯**: tap the row for the other actions.
- **Colors and counts.** A status is blue while an agent runs, green when it is Done, red when it failed or was interrupted, and grey when it is idle, stopped or archived. Amber is only for what needs your answer or click, for example a question or Quota paused. A small amber dot after a status marks an agent you have not seen yet. Counts that need you are amber pills (Today, Inbox); other counts are plain grey numbers.
- **Dates.** Past moments read “just now”, “5 min ago”, “3 h ago”, “yesterday” or “Oct 3”. Exact and future moments read “today 15:00”, “tomorrow 03:00” or “Fri 7 Nov, 03:00”, in your browser's time format, without seconds. Schedules read as sentences, for example “Weekdays at 09:00”; hover them to see the cron.
- **Page explanations.** A page explains itself under its title on your first visit on this device. After that, click **ⓘ** next to the title to read it again. The line under the title shows facts, such as counts.
- **Breadcrumbs.** On a laptop, the top bar shows where a detail page belongs: **Folders / folder / Local PR**, **Folders / folder / Pull requests / #N**, or **Inbox / owner/repo#N** for a GitHub item without a folder, and **Bots / bot**. Click a part to open that page.
- **Settings search.** On a laptop, Settings replaces the sidebar with its pages (**Back to Launchpad** returns to the page you were on); the field at the top finds pages by name and by keyword: "telegram" finds Workspace (its Telegram bot) and "runner" finds Home and runners.

### Open Launchpad on your phone

Open **Settings → Devices → Create pairing link** and scan the QR code. See [Remote access and phones](remote-access.md).

## Troubleshooting

- **Logs**: `journalctl --user -u launchpad -f` for a self-hosted Home, `journalctl --user -u launchpad-runner -f` for a runner installed by Home's script. Bots with admin access read the logs of Home and every runner (`launchpad logs`).
- **Status**: `launchpad status` prints JSON (add `LAUNCHPAD_ROLE=runner` for a runner).
- **Sign a browser in**: `launchpad url` on Home's machine prints a link that signs the browser in.
- **Restarting a runner** interrupts its running turns. Interrupted sessions can be resumed with one click, on that runner or another one.
