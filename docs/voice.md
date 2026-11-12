# Voice

Speak instead of typing. Every message box has a mic just left of **Send**. Your words show while you speak, then land in the box at the caret. Nothing is sent until you send it.

Voice uses your Claude subscription logins. Home sends the audio to Anthropic's speech service, the same one Claude Code's own voice mode uses.

## Where the mic is

You find it in five message boxes:

- An agent's message box: the session page, a Today row and **Reply** on the phone.
- The launch composer, in the row with **Attach**.
- A bot's chat, and the Ask panel.
- Channels and bot DMs.
- The top-bar field, at its right edge.

One dictation runs at a time in a window. When **Voice input** is Off in Settings → Voice, the mic is hidden everywhere. When voice is on but cannot work, the mic shows dimmed, and its tooltip says why.

## Dictate

- **Tap the mic** to start. Tap it again to stop.
- **Stay quiet** and it stops by itself. After a pause (8 s by default), a 2 s countdown ring shows on the stop button. Speak again and the countdown ends. A dictation always stops after 5 minutes.
- **Long press the mic** on a phone (350 ms or more) to talk while you hold it. It stops when you let go, and it never stops on silence.
- **Ctrl+Space** in a message box: a short tap starts or stops, like the mic. Hold it for 300 ms or more to talk until you release it.
- **Esc** stops the dictation.
- **Stop & send** replaces **Send** while you dictate. It stops, waits for your last words, then sends. In the top-bar field, **Ask** does the same. In the composer, **Launch** also waits for the last words.

While you dictate, a line over the box shows a level meter, **Connecting…** then **Listening** with the source's name, the time, and the current phrase in italics. Each finished phrase goes into the box at the caret, with spaces and a capital letter where they belong. Undo removes one phrase at a time. You can type and move the caret while you speak.

In a channel, a spoken member name becomes a mention (for example “Docs” becomes `@docs`) when it matches exactly. The names of the workspace's bots and folders, and a channel's members, go to the speech service as hints, so it spells them right.

Your text is safe:

- On a phone, the keyboard closes while you dictate, so you see the text. Android back and **‹** stop the dictation and keep the text.
- When the box closes before the last words arrive (back, **Hide**, another page), they go into the box's draft.

## Settings → Voice

**Settings → Voice** (also in the phone's Settings list, under **On the phone**):

- **Voice input**: On or Off for every device.
- **Sources, in order**: the Claude subscription profiles that voice uses, each with its login and state. Move them with ↑ and ↓, or remove them with ✕ (on the phone, these are in **⋯**). **Add** puts a profile back in the list. Two profiles with the same login count as one source. Without a choice, every Claude subscription profile is used, in the order of Settings → Profiles. Codex, OpenCode and pay-as-you-go profiles show as **Not supported for voice yet**.
- **Test voice**: records 3 seconds with each ready source in turn. It shows what each one heard and how soon the first words came.
- **Language**: the language you speak (English by default). The service does not detect it.
- **Stop after silence**: 5 s, 8 s (default), 15 s or Never.
- **This device**: the microphone (names show once the site may use it) and the start and stop sounds. These choices stay in this browser.

The shared choices are saved in `config.toml` under `[voice]` (see [Configuration](configuration.md#configtoml)).

A source's state:

| State | Meaning |
| --- | --- |
| **Ready** | Voice uses it. |
| **Waiting** | Its login used up a quota window. It is skipped until the time shown. |
| **Sign in again** | The login's quota check was refused. Sign in again in Settings → Credentials and tools. |
| **No login** | Home's vault has no Claude login for this profile. |

## Failover

Voice starts with the first ready source. When that source refuses the login, fails, closes the call or does not connect within 3 seconds, Home moves to the next one. It plays again what you said since the last finished phrase, so no words are lost. A quiet line says so, for example “Claude Code (Work): this login can't use voice. Continued with Claude Code (Personal); nothing lost.”

## Requirements

- **HTTPS.** Browsers allow the microphone only on HTTPS pages (and on `localhost`). Open Launchpad through its Tailscale or Space address (see [Remote access](remote-access.md)).
- **A Claude subscription login** in Home's vault, used by a Claude Code subscription profile.
- **Home reaches `api.anthropic.com` directly.** Voice ignores `HTTPS_PROXY`, so a Home behind a proxy cannot use it.
- **A browser window of Home.** The Chrome extension and a runner's own window have no mic.

## Errors

An error shows as one red line over the box, with **✕** to dismiss it.

| Message | What to do |
| --- | --- |
| Voice needs HTTPS | Open Launchpad through its HTTPS address. |
| Microphone blocked for this site | Allow the microphone in the browser's site settings (the icon left of the address), then try again. |
| No microphone found | Plug one in, or pick another in Settings → Voice. |
| The microphone is used by another app | Close the other app's call, then try again. |
| *Source*: this login can't use voice | Anthropic refused this login for voice. Voice tried the next source. See the known limits below. |
| Every voice login is used up | Every source used up a quota window. The mic works again at the time shown. |
| No voice source | Add a Claude subscription profile in Settings → Profiles. |
| No voice source is ready | Check the logins in Settings → Voice. |
| No voice source answered | Every source failed. The line gives the last reason. |
| Lost the connection to Home | The window lost its link to Home. The text already inserted stays. |

## Privacy

- The audio goes from your browser to Home, and from Home to Anthropic. Runners, Burst and agents never see it.
- Home keeps only the audio since the last finished phrase, in memory, so it can switch sources. It never stores audio or text, and its logs hold only sizes, times and HTTP statuses.
- Home uses its own copy of the login from the vault. It reads the token and never refreshes it.
- Agents cannot use voice. Only a signed-in browser window of Home opens a dictation; the CLI, agents' tokens and runners are refused.
- When you focus a message box or point at the mic, Home opens a connection to the speech service ahead of time (for 30 seconds at most), so the first words come faster. It sends no audio until you start.

## Known limits

- Only Claude subscription logins work. Codex refuses transcription on ChatGPT logins, and pay-as-you-go profiles are not supported.
- It is not known whether a login made with `claude setup-token` may use voice. If the speech service refuses it, the error line says “this login can't use voice”, and voice tries the next source.
- It is not known whether voice counts against a login's 5-hour and weekly windows.
- A Home that reaches the internet only through an HTTPS proxy cannot use voice.
- A dictation lasts 5 minutes at most.
- The language is fixed in Settings; the service does not detect it.
- In the demo, the mic plays a scripted voice. Nothing is recorded.
