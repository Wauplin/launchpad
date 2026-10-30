# Launchpad on Hugging Face Spaces

**Experimental components, not a deployable Launchpad runtime.** Startup intentionally keeps Launchpad stopped. Do not publish this directory as a working template or enter real provider credentials into it yet.

The [approved technical report](https://wauplin-launchpad-spaces-technical-report.static.hf.space) defines a private Docker Space, one machine, browser setup, readable bucket backups, encrypted credentials and deliberate updates. All HF policy and lifecycle code belongs here. Launchpad itself only gains a generic optional trace destination prefix; its default authentication, desktop packaging and trace formats stay as before.

## Implemented

- `supervisor/policy.py`: exact deployment identity/origin validation and startup-only Space/bucket privacy checks. Public, protected, absent or ambiguous visibility fails closed. No polling, visibility mutations, fencing or replica leases.
- `gateway/proxy.py`: reusable streaming bearer proxy for HTTP, bus and terminal WebSockets. Validates Host and mutation/upgrade Origin, rejects cross-site browser requests and ambiguous paths, strips supplied credentials and proxy identity headers, blocks pairing/devices/desktop/push routes, and never sends the internal bearer to a browser.
- `gateway/shell.html` and `shell.js`: responsive startup/locked screen, system light/dark appearance, keyboard focus, status refresh, Hub Settings links and compact Space controls. No credentials, local storage, analytics or daemon requests. The iframe integration is reserved for subsequent admission work; this supervisor never reports ready.
- `vault/codec.py`: AES-256-GCM credential envelopes with fresh nonces and HKDF keys per generation. Installation, format, generation, object kind and key identity are authenticated. Wrong keys and corrupt objects raise; nothing overwrites an old vault automatically.
- `backup/store.py`: immutable backup and credential generations with read-back verification, checksums, manifest-last commit and a latest hint. Existing generations are retained. Restore validates every object before returning data; it does not write to the runtime home.
- `backup/snapshot.py`: SQLite online backup, explicit schema projector and VACUUM/integrity checks. No raw database or full-home upload fallback.
- General Launchpad config: `trace_backup.prefix` (empty by default), config PATCH, Settings editor and demo fixtures. The Space's eventual configuration is bucket `namespace/name` plus prefix `traces`, never a bucket ID containing `/traces`.

## Current blocker

HF's normal Space-info API reports `private: true` for a protected Space, although protected apps are public. On 2026-10-31, requesting `expand[]=visibility` or `expand[]=protected` returned HTTP 400; the API listed neither as supported. The startup verifier currently requires an explicit `visibility: "private"` field and therefore deliberately refuses the current ambiguous response. This is an unresolved platform contract, not a missing token or a reason to weaken authentication. See [platform evidence and remaining gates](docs/acceptance.md).

Do not replace this with `private == true`, an owner checkbox or an undocumented fallback. An alternative ingress probe needs an explicitly reviewed acceptance contract and actual private/protected deployment checks.

## Read-only checks

Use Python 3.12 or newer with the exact dependencies in `requirements.txt`. Create a virtual environment outside the agent home. Set `SPACE_ID=namespace/name`, `SPACE_HOST=exact-subdomain.hf.space`, `LP_SPACE_BUCKET=namespace/name`, and a narrowly scoped `LP_SPACE_MANAGEMENT_TOKEN` in the supervisor environment. Never put the token in the repository, a command argument or browser storage.

From this directory:

```sh
python -m supervisor.probe
python -m supervisor.main
```

The first command reads metadata only and exits nonzero if privacy is unconfirmed. The second exposes the locked shell on port 7860, never starts agents, and never writes to the bucket. Use the configured HTTPS origin behind a private ingress; its Host checks deliberately reject direct localhost browsing. Status refresh reads the saved startup result and never polls HF visibility. Restart is required to verify a changed destination or policy.

`Proxy` is a library component; the locked supervisor intentionally does not attach it. Do not attach it to a real daemon until privacy, process isolation, recovery and deployment capability gates pass.

## Maintenance boundary

Keep the control plane independently versioned and packaged. Provider versions and credential extractors belong to this directory; harness execution, quota monitoring, profiles, sessions, terminals and trace export belong to Launchpad. Add a generic capability only when the adapter needs a documented core contract. Never import Space-specific code into Rust auth or the React shell, rewrite proxied app HTML, reuse the management token as agent HF auth, or auto-update binaries inside a running machine.

No production daemon, installed app, HF resource or provider login was changed while building these components. These files are local and have not been deployed.
