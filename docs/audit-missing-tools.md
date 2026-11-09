# Missing tools in past agent runs

Audit date: 2026-11-09. This is a historical audit only; it contains no implementation plan.

The traces support the concern. Missing dependencies caused skipped checks, repeated setup, temporary tool installations and improvised substitutes. The clearest recurring loss is the prose check: **6 separate sessions failed to import `textstat` and continued without a successful prose check in the retrieved trace**. Some other apparent tool gaps were actually package download failures, an incomplete Node installation or a crashing browser.

## Coverage and method

Queried Launchpad's session context both for Work and without a workspace filter. The latter returned 47 sessions: this audit, 43 other Work sessions and three outside Work. Retrieved all 46 past-session traces through the authorized `launchpad trace` CLI: **45 succeeded; one failed with `invalid trace sequence`** (`f3c9ff02-9aac-4236-9760-183d925553df`). About 10.8 MB of returned text was searched. Listed session start dates span November 6–9, 2026 (UTC).

Searched all retrieved text, including returned child-agent events, for missing commands/modules/packages, skipped checks, installation commands, browser failures and dependency recovery. Inspected surrounding commands, results and agent decisions for the cases below. Excluded matches that were merely source code, documentation, quoted examples or remote CI output. Counted sessions rather than repeated reports of the same failure across rebases. This is a search-based audit, not a claim that every command in 10.8 MB was manually reviewed.

The companion CSV lists every discovered session and whether its trace was retrieved. Evidence below uses session prefixes and event sequence numbers; resolve full IDs in that CSV and retrieve with `launchpad trace <full-id> --through <sequence>`. Raw traces stay outside the repository because they contain unrelated private conversation and service output.

Limits: this covers all sessions returned by the current context API, across workspaces, not deleted history or independent native sessions absent from Launchpad. Active sessions are snapshots. A missing tool inside a session does not establish whether it was absent on its host, hidden by the jail, absent on a Burst image, or missing from a newly created virtual environment. No current sandbox inventory was taken. Setup elapsed time was not reliably recorded, so no aggregate time estimate is claimed.

## Confirmed missing tools and agent decisions

| Tool / dependency | Sessions and evidence | Decision and observed outcome |
| --- | --- | --- |
| Python `textstat` | 6 sessions listed below | The checker stopped at import time. No successful rerun or dependency installation is visible afterward; work continued. |
| `pnpm` executable | `3d4b35fd` events 24–27; `9310777f` events 35–38 | Shell reported command not found. Agents invoked `corepack pnpm`, installed web dependencies and proceeded with typecheck/UI checks. These were recoverable command-availability gaps. |
| `strings` | `5010c934` events 32–34; `2522ff04` events 43–47 | Agents inspecting the Launchpad binary replaced `strings` with binary-mode `grep`. In the second case the first substitute hit a regex complexity error; a C-locale grep command then returned strings. No install; investigation continued with extra commands. |
| `jq` | `b362959b` events 27–29 and later child output | PR and Hub data queries failed with command not found. Agent wrote a Python JSON-processing/listing script; it successfully listed 200 PRs. Later child commands repeated the missing-jq error. Work continued using Python/gh JSON facilities; recovery did not prevent repeated failed attempts. |
| Rust toolchain and C/C++ compiler | `c70046a8` events 50–58, 105 onward, 286 onward | On Burst: no Rust toolchain, no C compiler and no root. Installed Rust 1.93.1/Clippy with rustup; installed Python `ziglang` and wrote compiler/archive wrappers. Repeated setup on later fresh machines in the same session. Builds/tests eventually ran; later Clippy reached an unrelated existing `netcap.rs` lint. This is the strongest explicit repeated-install case. |
| Python `aiohttp` | `5fd8ffe6` events around the test import failure and final report; `63cd79c2` child events 972 and 1042 | First agent left the hf-home tests unrun, explaining the dependency was absent and the tests did not cover its shell-script change. In the second session a child installed requirements in `/tmp/hfhome-venv` and reported **15 tests passed**. Same missing module, different decisions. |
| Chrome | `9310777f` events 44–45, final reports | Screenshot script reported Chrome not found. Agent continued with typecheck/hooks/UI checks, disclosed that browser validation was unavailable. |

### Prose-check failures, individually

- `31dd8159` (Attribution footer automation for Launchpad agents, rebased onto main), event 97: Explicitly said the check was skipped/unavailable and continued to commit.
- `350e67f9` (Fix headless Chrome crashes in Launchpad agent sandboxes), event 212: Explicitly said the check was skipped/unavailable and continued to commit.
- `3a281ad5` (Add auto-update for Launchpad Home Space with runner follow-up), event 133: Continued to other work and committed; no successful prose-check rerun found.
- `9310777f` (Today: add an inline arrow to expand agent last messages and move Archive beside Open), event 54: Explicitly said the check was skipped/unavailable and continued to commit.
- `a1a04c15` (Fix Burst placeholder relaunch and failure reasons), event 65: Explicitly said the check was skipped/unavailable and continued to commit.
- `e73cad9f` (Admin session: rebase onto latest main and re-request merge, plus CLI diagnostics), event 237: Continued to other work and committed; no successful prose-check rerun found.

The checker failed at `import textstat`. This does **not** prove that `proselint` or its other dependencies were present: execution never reached those checks.

## Package-manager, project-dependency and runtime detours

These matter to the audit but should not all be counted as missing base tools.

| Sessions | Failure and action | Outcome |
| --- | --- | --- |
| `72336c6b` events 40–50 | Corepack failed; attempted npx, then installed pnpm 10.33.0 into `/tmp/lp-pnpm`. | Dependency install and typecheck succeeded; agent reported tests passing. Browser still crashed. |
| `d234cd3d` events 22–39 | Registry/Corepack attempts failed; downloaded a pnpm tarball with curl, extracted it and created a temporary launcher. | Got past package-manager setup and continued UI investigation; no code change was needed. |
| `350e67f9` events 160–171 | Investigated registry failure; identified PATH Node as a partial installation with only `bin` mounted. Switched to nvm Node 24. | Retried dependency installation/build with the complete Node installation. This was not proof pnpm itself was absent. |
| `94a2fb5c` events 42–51 | Screenshot/build attempt lacked web dependencies; Corepack failed. Switched to nvm Node 24 with CA configuration. | Built UI; browser execution remained unavailable, so agent reasoned from code. |
| `38238ff4` | Tried npm setup because pnpm was not cached; ultimately borrowed another worktree's `node_modules`. | Typecheck and hooks lint passed; temporary link removed. |
| `3a281ad5` | pnpm download blocked; borrowed another worktree's `node_modules` with the same lockfile. | Typecheck, hooks lint and UI check passed. Prose check still failed separately. |
| `8b76d73e` | pnpm could not download; borrowed another worktree's dependencies. | Web typecheck passed. |
| `e73cad9f` | Initially reviewed fixtures manually when package download blocked. A later turn used another worktree's dependencies. | Initially unverified web typecheck; later `tsc -b` passed. |
| `5d754417` | Corepack and npm both failed while obtaining web dependencies. | Left web typecheck unrun; reviewed diff by hand and disclosed limitation. |
| `3e2f5e40` | Dependency install blocked by network; searched for local TypeScript. | Final report left web typecheck and visual review unverified. Rust Clippy passed. |
| `55e4846e` | Dependencies absent (`@tanstack/react-query` import failure), registry download failed. | Typecheck, hooks lint and session tests blocked; UI style and diff checks passed. Continued to commit with limitations disclosed. |
| `8f75cfdf` events 10–21 and subsequent checks | Fresh Python venv lacked click/typer. Initial install commands had shell interpolation/PEP508 errors; corrected command installed quality extras, then testing extras and click 8.5.1. | Reproduced target type-check bug; ran Ruff, mypy and pytest. Missing yaml/toml typing stubs also appeared in mypy output. These are project-environment/setup issues, not evidence every jail lacked click. |
| `1241f2d8` events 56 onward | Installed omp dependencies to query bundled model data. Existing Bun failed; installed newer Bun in `/tmp/newbun`. Then native `pi_natives` modules were missing. | Further attempts at executing the catalog failed; audit proceeded from source/catalog and API evidence. This was a project/native-dependency and runtime-version detour. |

## Related failures that are not missing tools

- **Installed Chrome crashed:** `94a2fb5c`, `72336c6b`, `f2ff3f5e` and the dedicated investigation `350e67f9` found `google-chrome` present. Missing alternative names (`chromium`, `chrome`) in a multi-name probe do not establish no browser existed. Some agents abandoned screenshots and used code inspection. The investigation attributed crashes to network-capture process handling; its test build worked with Chrome/Playwright, with a separate user-namespace restriction on HF Jobs. This audit does not revalidate that diagnosis.
- **sccache timed out:** `31dd8159` and other Rust runs bypassed it with `RUSTC_WRAPPER=` and compiled without caching. It was present but unusable, not a missing executable.
- **Shell command error:** `command not found: all` in `63cd79c2` came from command construction; it is not a tool named `all` to add.
- Installing a checkout's `node_modules` or Python testing extras is recorded above as setup, but does not by itself establish a missing general-purpose tool.

## Common tools worth considering

This is a candidate inventory, not a claim that these tools are currently absent and not an implementation proposal.

The strongest trace-backed candidates are **jq, binutils (`strings`), a complete Rust toolchain with Clippy, a C/C++ compiler and archiver, usable Node/pnpm, `textstat`, and a working headless browser**. `aiohttp` is also directly relevant to this repository's Python tests. A working browser and package manager involve more than the executable existing, as the traces show.

Other common candidates, with no confirmed absence established in this audit:

| Area | Candidates | Why useful |
| --- | --- | --- |
| Shell and files | bash, zsh, coreutils, findutils, grep, sed, awk, ripgrep, file, xxd, patch, diff, tar, gzip, xz, zip/unzip | Search, inspect files, apply patches and unpack artifacts. |
| Git and network | git, curl, wget, CA certificates, openssh-client, DNS lookup tools | Fetch source/artifacts and diagnose connectivity. Availability does not imply credentials or permission. |
| Build tools | make, CMake, Ninja, pkg-config, GCC/Clang, binutils | Common native builds and Python/Node native extensions. [GNU binutils](https://www.gnu.org/software/binutils/) includes binary inspection tools such as readelf. |
| Python | Python 3, venv, pip, uv, pytest, Ruff; project-required typing tools | Common environment, test and lint workflows. [pytest documentation](https://docs.pytest.org/en/stable/) describes its testing role. |
| Shell/docs checks | ShellCheck, shfmt, markdownlint, proselint, textstat | Avoid bypassing routine shell/docs validation. [ShellCheck](https://www.shellcheck.net/) analyzes shell scripts. Only textstat's absence was confirmed here. |
| Data/debugging | sqlite3 CLI, jq, a YAML processor, lsof, ss, ps, strace | Inspect local data and diagnose processes, sockets and failures. Only jq's absence was confirmed here. |
| UI/media | Headless Chrome/Chromium, Playwright and browser dependencies/fonts; Pillow or ImageMagick for image inspection | Visual checks and screenshot/image workflows. No claim that all are needed for every task. |

No application code, sandbox configuration or production settings were changed. Audit validation consisted of checking retrieval coverage, matching failures to subsequent actions, and reviewing this report's evidence references. No builds or product tests are applicable to this documentation-only audit.
