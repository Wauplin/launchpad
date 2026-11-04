# Home and runners

Work in progress: this is the first part of the [Home and runners plan](https://wauplin-launchpad-bots-research.static.hf.space/index.html). Today's single-process mode (`launchpad daemon`, role `all`) is unchanged.

- **Home** (`launchpad daemon --role home`) keeps every record and runs no agents or terminals.
- **Runners** (`launchpad daemon --role runner`) run agents. A runner works in its own folders (`~/.local/share/launchpad-runner`, `~/.config/launchpad-runner`) and never reuses a single-process install's data.

## Pair a runner

1. Start the runner daemon on the machine: `launchpad daemon --role runner`.
2. Run `launchpad connect <Home address>` there. It prints a code such as `34Z-K9Z` and waits. For a Home in a private Hugging Face Space, the runner sends your HF token (`HF_TOKEN` or the `hf auth login` token) to pass the Space's sign-in.
3. On Home, open **Settings → Home and runners**, check that the code matches and click **Approve**. Requests expire after ten minutes.

The runner keeps one outbound WebSocket to Home (no inbound port) and reconnects on its own. **Remove** on Home disconnects a runner; it must pair again.

## Versions

Home and its runners compare their protocol and build. A runner with another protocol cannot connect. A runner with another build of the same protocol shows **Update this runner** and takes no new sessions; its running sessions continue.

## Runner guard

On a runner, **Settings → This runner** holds settings that never sync to Home: **Launches from other devices** (Accept or Refuse) and **Pause runner**. Home shows them next to the runner. They are conveniences, not a security boundary: runners trust Home.
