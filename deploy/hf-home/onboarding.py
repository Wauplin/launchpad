"""Home's onboarding: the steps between an empty Space and a first agent.

Required before Home opens: Hugging Face, the credential key, GitHub, an SSH key, the Git identity, a profile and a
first folder. Then, optional on this page: pair a runner and run a first agent.

Only fixed CLI logins run, and only after an owner click. Their output stays in memory and is never logged.
Credentials are encrypted with a key derived from `LP_HOME_KEY` (a Space secret the owner chooses or generates here):
Home's own logins in the bucket (decrypted only into this container's local disk) and the vault secrets in Home's
backups. Everything else is stored plain.
"""
import asyncio
import base64
import errno
import hashlib
import hmac
import os
from pathlib import Path
import pty
import re
import secrets
import shutil
import signal
import tempfile
import time

import aiohttp
from aiohttp import web
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

ANSI = re.compile(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_]|[78])")
# OSC 8 hyperlinks carry the whole URL even when the visible text is cut at the terminal width.
OSC8 = re.compile(r"\x1b\]8;[^;\x07\x1b]*;(https://[^\x07\x1b]+)(?:\x07|\x1b\\)")
URL = re.compile(r"https://[^\s<>\"'\x1b\x07]+")
# One-time device codes printed by hf ("ABCD-EFGH"), gh ("ABCD-1234") and Codex ("ABCD-12345").
CODE = re.compile(r"\b[A-Z0-9]{4}-[A-Z0-9]{4,5}\b")
# Claude Code's long-lived token (any `sk-ant-oat<version>-` prefix), then the line printed after it.
CLAUDE_TOKEN = re.compile(r"sk-ant-oat\d*-[A-Za-z0-9_-]{20,}")
CLAUDE_TOKEN_END = re.compile(r"Store\s*this\s*token")
# How long Claude Code may take to exchange a pasted code before the page says so.
CODE_WAIT = 90
# Claude Code's answer to a rejected code; cursor moves may replace the spaces.
RETRY = re.compile(r"Press\s*Enter\s*to\s*retry")
TIMEOUT = 900
MAGIC = b"LPSEALED1"
KEY_SECRET = "LP_HOME_KEY"
MIN_KEY = 12
LOGINS = {"claude": "claude_login", "codex": "codex_login"}  # Harnesses whose login Home keeps in its vault.
NAMES = {"claude": "Claude Code", "codex": "Codex"}
# Owner choices kept in Home's onboarding record (`dismissed`), so they survive restarts and restores.
SSH_DONE, PROFILE_DONE, AGENT_DONE = "home:ssh", "home:profile", "home:first-agent"
FIRST_PROMPT = "What is this project about? Give a short overview of its purpose, structure and main parts."


def device_prompt(output):
    """The last sign-in link and one-time code printed by a login."""
    links = OSC8.findall(output)
    text = ANSI.sub("", output).replace("\r", "")
    links = links or [u.rstrip(".,;:)") for u in URL.findall(text)]
    codes = CODE.findall(text)
    return (links[-1] if links else None), (codes[-1] if codes else None)


def claude_token(output, complete=False):
    """The token Claude Code printed, once all of it arrived: the line after it is there, or the process exited
    (`complete`). A token wrapped over several lines is joined."""
    text = ANSI.sub("", output)
    start = text.rfind("sk-ant-oat")
    if start < 0:
        return None
    rest = text[start:]
    end = CLAUDE_TOKEN_END.search(rest)
    if end:
        found = CLAUDE_TOKEN.match(re.sub(r"\s+", "", rest[:end.start()]))
    elif complete:
        found = CLAUDE_TOKEN.match(rest)
    else:
        return None
    return found.group(0) if found else None


def last_line(output):
    """The last visible line of a sign-in terminal, without anything that looks like a token."""
    lines = [line.strip() for line in ANSI.sub("", output).replace("\r", "\n").split("\n")]
    line = next((line for line in reversed(lines) if line), "")
    return re.sub(r"sk-ant-\S+", "…", line)[:300]


def unique_id(base, taken):
    value, n = base, 2
    while value in taken:
        value, n = f"{base}-{n}", n + 1
    return value


def derive_key(passphrase):
    """The 32-byte AES key from the LP_HOME_KEY passphrase (scrypt with a fixed salt: the passphrase is the secret)."""
    value = passphrase.strip()
    if len(value) < MIN_KEY:
        return None
    return hashlib.scrypt(value.encode(), salt=b"launchpad-home-key", n=1 << 15, r=8, p=1, maxmem=1 << 26, dklen=32)


def new_passphrase():
    return "lp-" + secrets.token_urlsafe(30)


def seal(key, data):
    """MAGIC || nonce || AES-256-GCM ciphertext and tag."""
    nonce = os.urandom(12)
    return MAGIC + nonce + AESGCM(key).encrypt(nonce, data, None)


def unseal(key, data):
    if not data.startswith(MAGIC):
        raise ValueError("Not an encrypted file")
    rest = data[len(MAGIC):]
    return AESGCM(key).decrypt(rest[:12], rest[12:], None)


def write_private(path, data):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(data)
    temporary.chmod(0o600)
    temporary.replace(path)


def noreply_email(user):
    return f"{user['id']}+{user['login']}@users.noreply.github.com" if user.get("id") and user.get("login") else ""


class Login:
    """One fixed login process. `done(output)` runs after a successful exit, or as soon as `ready(output)` is true."""

    def __init__(self, tool, command, env, done, cleanup=None, terminal=False, ready=None):
        self.tool, self.command, self.env, self.done = tool, command, env, done
        self.cleanup, self.terminal, self.ready = cleanup, terminal, ready
        self.process = None
        self.task = None
        self.master = None
        self.output = b""
        self.error = None
        self.cancelled = False
        self.code_sent = False
        self.code_at = 0
        self.watchdog = None

    @property
    def running(self):
        return bool(self.task and not self.task.done())

    async def start(self):
        self.output, self.error, self.cancelled = b"", None, False
        if self.terminal:
            # Claude Code's sign-in is a terminal UI: give it a wide PTY so links and tokens stay on one line.
            import fcntl, struct, termios
            master, slave = pty.openpty()
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 500, 0, 0))
            try:
                self.process = await asyncio.create_subprocess_exec(
                    *self.command, env=self.env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            except BaseException:
                os.close(master)
                raise
            finally:
                os.close(slave)
            self.master = master
        else:
            self.process = await asyncio.create_subprocess_exec(
                *self.command, env=self.env, stdin=asyncio.subprocess.DEVNULL, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT, start_new_session=True)
        self.task = asyncio.create_task(self.read())

    async def chunk(self):
        if self.master is None:
            return await self.process.stdout.read(4096)
        try:
            return await asyncio.to_thread(os.read, self.master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                return b""  # The process closed its terminal.
            raise

    async def read(self):
        async def pump():
            while data := await self.chunk():
                self.output = (self.output + data)[-262144:]
                if self.ready and self.ready(self.output.decode(errors="replace")):
                    await self.done(self.output.decode(errors="replace"))
                    await self.stop()
                    return None
                if self.code_sent and RETRY.search(ANSI.sub("", self.output[self.code_at:].decode(errors="replace"))):
                    self.error = "That code was not accepted. Start the sign-in again and paste the newest code."
                    await self.stop()
                    return None
            return await self.process.wait()

        try:
            code = await asyncio.wait_for(pump(), TIMEOUT)
            if code == 0:
                await self.done(self.output.decode(errors="replace"))
            elif code is not None and not self.cancelled and not self.error:
                self.error = "The sign-in did not complete. Start it again."
        except asyncio.TimeoutError:
            await self.stop()
            self.error = "The sign-in expired. Start it again."
        except Exception as error:  # Never log the output: it can hold codes and tokens.
            await self.stop()
            self.error = str(error) or "The sign-in could not be saved."
        finally:
            if self.watchdog:
                self.watchdog.cancel()
            self.output = b""
            if self.master is not None:
                os.close(self.master)
                self.master = None
            if self.cleanup:
                self.cleanup()

    def send_code(self, code):
        if self.master is None or not self.running:
            raise ValueError("Start the sign-in again")
        self.code_at = len(self.output)
        os.write(self.master, code.encode() + b"\r")
        self.code_sent = True
        if self.watchdog:
            self.watchdog.cancel()
        self.watchdog = asyncio.get_running_loop().create_task(self.wait_for_token())

    async def wait_for_token(self):
        """A pasted code that Claude Code neither accepts nor rejects in time ends the sign-in with what it shows."""
        await asyncio.sleep(CODE_WAIT)
        if self.running:
            shown = last_line(self.output[self.code_at:].decode(errors="replace"))
            self.error = "Claude Code did not finish the sign-in" + (f" (it shows: {shown})" if shown else "") + ". Start it again."
            await self.stop()

    async def stop(self):
        if self.process and self.process.returncode is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
                await asyncio.wait_for(self.process.wait(), 10)
            except ProcessLookupError:
                pass
            except asyncio.TimeoutError:
                os.killpg(self.process.pid, signal.SIGKILL)
                await self.process.wait()

    def status(self):
        link, code = device_prompt(self.output.decode(errors="replace"))
        return {"tool": self.tool, "running": self.running, "link": link, "code": None if self.terminal else code,
                "needs_code": self.terminal and bool(link) and not self.code_sent, "error": self.error}


class Setup:
    def __init__(self, space, host, bucket, credentials, key):
        self.space, self.host, self.bucket = space, host, bucket
        self.key = key  # Derived from the LP_HOME_KEY Space secret; None until the owner sets it here.
        self.csrf = secrets.token_hex(32)
        self.sealed = credentials / "sealed"
        self.hf_home = Path(os.environ["HF_HOME"])
        self.gh_dir = Path(os.environ["GH_CONFIG_DIR"])
        self.files = {"huggingface": (self.hf_home / "token", self.sealed / "huggingface-token"),
                      "github": (self.gh_dir / "hosts.yml", self.sealed / "gh-hosts.yml")}
        self.hf = {"done": False, "account": None, "error": None}
        self.github = {"done": False, "account": None, "error": None, "user": {}}
        # "off": no key yet; "restarting": the owner saved a key and Home restarts to use it; "manual": the owner
        # must add the secret in the Space settings.
        self.encryption = {"state": "on" if key else "off", "error": None}
        self.passphrase = None  # A key generated or typed on this page, until Home restarts with it.
        self.home = {}  # What Home's daemon has: SSH key, identity, profiles, folders, runners.
        self.home_at = 0.0
        self.login = None
        self.lock = asyncio.Lock()
        self.api = None  # Set once the daemon is up: async (method, path, body=None) -> JSON.
        self.binary = Path(shutil.which("launchpad") or "/usr/local/bin/launchpad")
        self.binary_sha = None

    # ------------------------------------------------------------------ credentials at rest

    def restore(self):
        """Decrypt Home's own logins from the bucket into the container's disk."""
        if not self.key:
            return
        for local, sealed in self.files.values():
            if sealed.exists():
                write_private(local, unseal(self.key, sealed.read_bytes()))

    def save(self, name, key=None):
        key = key or self.key
        local, sealed = self.files[name]
        if key and local.exists():
            write_private(sealed, seal(key, local.read_bytes()))

    def hf_api(self):
        from huggingface_hub import HfApi, get_token
        return HfApi(token=get_token())

    async def save_key(self, passphrase):
        """Keep the key as a Space secret, then restart Home to use it. Runs only on the owner's Restart click."""
        if self.key:
            raise ValueError("Encryption is already on")
        if not self.hf["done"]:
            raise ValueError("Sign in to Hugging Face first")
        key = derive_key(passphrase)
        if key is None:
            raise ValueError(f"Use a key of at least {MIN_KEY} characters")
        self.passphrase = passphrase.strip()
        # Home's HF login is sealed with the new key now, so Home finds it after the restart.
        self.save("huggingface", key)
        api = self.hf_api()
        try:
            await asyncio.to_thread(api.add_space_secret, self.space, KEY_SECRET, self.passphrase,
                                    description="Encrypts Launchpad Home's credentials in the bucket. Never share or delete it.")
        except Exception:
            self.encryption = {"state": "manual", "error": None}
            return
        self.encryption = {"state": "restarting", "error": None}
        asyncio.get_running_loop().call_later(1, lambda: asyncio.ensure_future(self.restart()))

    async def restart(self):
        try:
            await asyncio.to_thread(self.hf_api().restart_space, self.space)
        except Exception:
            pass  # Saving a secret restarts the Space too.

    # ------------------------------------------------------------------ checks

    async def check_hf(self):
        from huggingface_hub import get_token
        token = get_token()
        if not token:
            self.hf = {"done": False, "account": None, "error": None}
            return
        try:
            async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as client:
                headers = {"Authorization": f"Bearer {token}"}
                async with client.get("https://huggingface.co/api/whoami-v2", headers=headers) as response:
                    response.raise_for_status()
                    account = (await response.json())["name"]
                async with client.get(f"https://huggingface.co/api/buckets/{self.bucket}", headers=headers) as response:
                    response.raise_for_status()
                    if (await response.json()).get("private") is not True:
                        raise ValueError("private")
            self.hf = {"done": True, "account": account, "error": None}
        except Exception:
            self.hf = {"done": False, "account": None, "error": f"Could not verify this login and its access to the private bucket {self.bucket}. Sign in with an account that can write to it."}

    async def gh(self, *args):
        process = await asyncio.create_subprocess_exec(
            "gh", *args, env=dict(os.environ, GH_PROMPT_DISABLED="1"), stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL)
        out, _ = await asyncio.wait_for(process.communicate(), 30)
        return out.decode().strip() if process.returncode == 0 else None

    async def check_github(self):
        try:
            # A stored token is enough (no network); the account details are best effort.
            stored = self.secret() or (self.gh_dir / "hosts.yml").exists()
            if not stored or not await self.gh("auth", "token", "--hostname", "github.com"):
                self.github = {"done": False, "account": None, "error": None, "user": {}}
                return
            user = {}
            raw = await self.gh("api", "user", "--jq", "[.login, .name // \"\", .id] | @tsv")
            if raw:
                login, name, uid = (raw.split("\t") + ["", "", ""])[:3]
                user = {"login": login, "name": name or login, "id": uid}
            self.github = {"done": True, "account": user.get("login"), "error": None, "user": user}
        except (OSError, asyncio.TimeoutError):
            self.github = {"done": False, "account": None, "error": "Could not check the GitHub login.", "user": {}}

    async def check_home(self, force=False):
        """Reads the daemon's side of the setup. Cheap local calls, at most every second."""
        if not self.api or not self.unlocked or (not force and time.monotonic() - self.home_at < 1):
            return
        self.home_at = time.monotonic()
        config = (await self.api("GET", "/api/config"))["config"]
        vault = await self.api("GET", "/api/vault")
        marks = set((await self.api("GET", "/api/onboarding")).get("dismissed", []))
        workspaces = await self.api("GET", "/api/workspaces")
        projects = await self.api("GET", "/api/projects")
        runners = await self.api("GET", "/api/runners")
        try:
            burst = await self.api("GET", "/api/burst")
        except Exception:
            burst = {"unavailable": "Burst is not available on this Home", "config": {"monthly_cap_usd": 0}, "month_usd": 0}
        entries = vault["entries"]
        ssh = next((e for e in entries if e["kind"] == "ssh_key"), None)
        profiles, missing = [], None
        for p in config["profiles"]:
            kind = LOGINS.get(p["harness"])
            ok = kind is None or any(e["kind"] == kind and (not p.get("login") or e["id"] == p["login"]) for e in entries)
            if ok:
                profiles.append({k: p.get(k) for k in ("id", "name", "harness", "model", "effort")})
            elif missing is None:
                missing = {"name": p["name"], "harness": p["harness"]}
        agent = AGENT_DONE in marks
        if not agent:
            agent = bool(await self.api("GET", "/api/sessions"))
        self.home = {
            "ssh": {"done": bool(ssh) and SSH_DONE in marks, "public_key": f"{ssh['detail']} launchpad-home" if ssh else None},
            "identity": {"done": bool(vault["git_name"].strip() and vault["git_email"].strip()), "name": vault["git_name"], "email": vault["git_email"]},
            "profile": {"done": bool(profiles) and PROFILE_DONE in marks, "ready": bool(profiles), "profiles": profiles, "missing": missing},
            "folders": {"done": bool(workspaces) and bool(projects), "workspaces": [{"id": w["id"], "name": w["name"]} for w in workspaces],
                        "projects": [{"id": p["id"], "name": p["name"], "workspace_id": p.get("workspace_id")} for p in projects]},
            # Agents can run once a runner is connected or Burst is available (an HF Job, nothing to install).
            "runner": {"done": any(r["connected"] for r in runners),
                       "runners": [{"id": r["runner"]["id"], "name": r["runner"]["name"], "code": r["runner"]["code"], "state": r["runner"]["state"],
                                    "connected": r["connected"], "compat": r.get("compat")} for r in runners],
                       "burst": {"available": burst.get("unavailable") is None, "unavailable": burst.get("unavailable"),
                                 "cap": burst.get("config", {}).get("monthly_cap_usd", 0)}},
            "agent": {"done": agent},
        }

    async def check(self):
        await self.check_hf()
        await self.check_github()
        try:
            await self.check_home(force=True)
        except Exception:
            pass

    # ------------------------------------------------------------------ state

    def secret(self):
        return bool(os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN"))

    @property
    def unlocked(self):
        """Home's data may be used: signed in to HF with encryption on."""
        return self.hf["done"] and self.key is not None

    def step(self, name):
        return self.home.get(name, {}).get("done", False)

    @property
    def complete(self):
        """Home opens once these are done; the runner and the first agent are optional here."""
        return self.unlocked and self.github["done"] and all(self.step(s) for s in ("ssh", "identity", "profile", "folders"))

    def install_command(self):
        url = f"https://{self.host}/_home/runner/install.sh"
        return f'curl -fsSL -H "Authorization: Bearer ${{HF_TOKEN:-$(cat ~/.cache/huggingface/token)}}" {url} | bash'

    def status(self):
        github = {k: v for k, v in self.github.items() if k != "user"}
        user = self.github.get("user") or {}
        return {
            "csrf": self.csrf, "bucket": self.bucket, "space": self.space, "complete": self.complete, "unlocked": self.unlocked,
            "huggingface": dict(self.hf, done=self.hf["done"], signed_in=self.hf["done"]),
            "key": {"done": self.key is not None, "state": self.encryption["state"], "error": self.encryption["error"],
                    "pending": self.passphrase is not None},
            "github": dict(github, secret=self.secret(), suggest={"name": user.get("name", ""), "email": noreply_email(user)}),
            **{k: self.home.get(k, {"done": False}) for k in ("ssh", "identity", "profile", "folders", "runner", "agent")},
            "install": self.install_command(),
            "first_prompt": FIRST_PROMPT,
            "login": self.login.status() if self.login else None,
        }

    # ------------------------------------------------------------------ actions

    async def start(self, tool):
        async with self.lock:
            if self.login and self.login.running:
                raise ValueError("Finish or cancel the current sign-in first")
            base = dict(os.environ, BROWSER="/bin/true", NO_COLOR="1")
            if tool == "huggingface":
                env = dict(base, HF_HUB_DISABLE_UPDATE_CHECK="1", PYTHONUNBUFFERED="1")
                self.login = Login(tool, ("hf", "auth", "login", "--force", "--format", "agent"), env, self.hf_done)
            elif not self.unlocked:
                raise ValueError("Finish the Hugging Face and key steps first")
            elif tool == "github":
                if self.secret():
                    raise ValueError("GitHub uses the GH_TOKEN Space secret; remove it in the Space settings to sign in here")
                env = {k: v for k, v in base.items() if k not in ("GH_TOKEN", "GITHUB_TOKEN")}
                env["GH_PROMPT_DISABLED"] = "1"
                command = ("gh", "auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web",
                           "--skip-ssh-key", "--insecure-storage", "--scopes", "repo,read:org,notifications,workflow")
                self.login = Login(tool, command, env, self.github_done)
            elif tool in ("codex", "claude"):
                if not self.api:
                    raise ValueError("Home is still starting. Try again in a moment.")
                # A throwaway home for the CLI: the login goes to the vault, then this folder is deleted.
                folder = Path.home() / ".local/state/home-setup" / secrets.token_hex(8)
                folder.mkdir(mode=0o700, parents=True)

                def cleanup():
                    shutil.rmtree(folder, ignore_errors=True)

                if tool == "codex":
                    (folder / "config.toml").write_text('cli_auth_credentials_store = "file"\n')

                    async def done(_):
                        await self.add_login("codex", (folder / "auth.json").read_text())

                    self.login = Login(tool, ("codex", "login", "--device-auth"), dict(base, CODEX_HOME=str(folder)), done, cleanup)
                else:
                    async def done(output):
                        token = claude_token(output, complete=True)
                        if not token:
                            raise ValueError("Claude Code did not print a token. Start the sign-in again.")
                        await self.add_login("claude", token)

                    env = dict(base, CLAUDE_CONFIG_DIR=str(folder), TERM="xterm-256color")
                    env.pop("NO_COLOR")
                    self.login = Login(tool, ("claude", "setup-token"), env, done, cleanup, terminal=True,
                                       ready=lambda out: claude_token(out) is not None)
            else:
                raise ValueError("Choose a supported sign-in")
            await self.login.start()

    async def hf_done(self, _):
        await self.check_hf()
        if self.hf["done"] and self.key:
            self.save("huggingface")

    async def github_done(self, _):
        self.save("github")
        await self.check_github()
        if self.github["done"] and self.api:
            try:
                await self.api("POST", "/api/inbox/sync", {})
            except Exception:
                pass  # The next poll syncs the inbox.

    async def add_login(self, harness, secret):
        """Store a harness login in Home's vault; a profile without its login gets it, otherwise a new profile."""
        if not self.api:
            raise ValueError("Home is still starting. Try again in a moment.")
        vault = await self.api("GET", "/api/vault")
        ids = {e["id"] for e in vault["entries"]}
        entry = unique_id(harness, ids)
        await self.api("PUT", f"/api/vault/entries/{entry}", {"kind": LOGINS[harness], "name": NAMES[harness], "value": secret})
        config = (await self.api("GET", "/api/config"))["config"]
        has_login = any(e["kind"] == LOGINS[harness] for e in vault["entries"])
        waiting = next((p for p in config["profiles"] if p["harness"] == harness
                        and (p.get("login") not in ids if p.get("login") else not has_login)), None)
        if waiting:
            waiting["login"] = entry
        else:
            config["profiles"].append({"id": unique_id(harness, {p["id"] for p in config["profiles"]}),
                                       "name": NAMES[harness], "harness": harness, "login": entry})
        await self.api("PUT", "/api/config", config)
        await self.check_home(force=True)

    async def create_ssh_key(self):
        """An ed25519 key pair made on Home. The private key goes to the vault (encrypted in backups) and never
        leaves Home: sessions use it through a forwarded ssh-agent."""
        vault = await self.api("GET", "/api/vault")
        if any(e["kind"] == "ssh_key" for e in vault["entries"]):
            return
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "id_ed25519"
            process = await asyncio.create_subprocess_exec(
                "ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "launchpad-home", "-f", str(path),
                stdin=asyncio.subprocess.DEVNULL, stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL)
            if await process.wait() != 0:
                raise ValueError("Could not create the SSH key")
            private = path.read_text()
        await self.api("PUT", "/api/vault/entries/ssh-key", {"kind": "ssh_key", "name": "Home SSH key", "value": private})

    async def set_profile(self, body):
        config = (await self.api("GET", "/api/config"))["config"]
        profile = next((p for p in config["profiles"] if p["id"] == body.get("id")), None)
        if not profile:
            raise ValueError("This profile no longer exists")
        for field in ("model", "effort"):
            value = body.get(field)
            profile[field] = str(value).strip()[:120] if value else None
        await self.api("PUT", "/api/config", config)

    async def mark(self, item):
        await self.api("PATCH", "/api/onboarding", {"dismiss": [item]})

    async def close(self):
        if self.login:
            await self.login.stop()
            if self.login.task:
                await asyncio.gather(self.login.task, return_exceptions=True)

    async def action(self, path, body):
        if path == "/_home/login":
            await self.start(body.get("tool"))
        elif path == "/_home/cancel":
            if self.login:
                self.login.cancelled = True
                await self.login.stop()
        elif path == "/_home/code":
            code = str(body.get("code", "")).strip()
            if not code or len(code) > 512 or not code.isprintable():
                raise ValueError("Paste the code shown after you approve")
            if not self.login or self.login.tool != "claude":
                raise ValueError("Start the Claude Code sign-in first")
            self.login.send_code(code)
        elif path == "/_home/check":
            await self.check()
        elif path == "/_home/key/new":
            # Sent once for the owner's clipboard or key file. The page never displays it.
            if self.key:
                raise ValueError("Encryption is already on")
            self.passphrase = new_passphrase()
            return {"passphrase": self.passphrase}
        elif path == "/_home/key/copy":
            if not self.passphrase:
                raise ValueError("Generate or enter a key first")
            return {"passphrase": self.passphrase}
        elif path == "/_home/key/save":
            await self.save_key(str(body.get("key") or self.passphrase or ""))
        elif path == "/_home/restart":
            if self.encryption["state"] not in ("restarting", "manual"):
                raise ValueError("Nothing needs a restart")
            self.encryption["state"] = "restarting"
            await self.restart()
        elif not self.unlocked or not self.api:
            raise ValueError("Finish the Hugging Face and key steps first")
        elif path == "/_home/ssh":
            await self.create_ssh_key()
        elif path == "/_home/ssh/done":
            await self.mark(SSH_DONE)
        elif path == "/_home/profile":
            await self.set_profile(body)
            await self.mark(PROFILE_DONE)
        elif path == "/_home/agent":
            await self.mark(AGENT_DONE)
        else:
            raise web.HTTPNotFound()
        await self.check_home(force=True)
        return None

    # ------------------------------------------------------------------ runner install

    def install_script(self, commit):
        if self.binary_sha is None:
            self.binary_sha = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        text = Path(__file__).with_name("install-runner.sh").read_text()
        for name, value in (("HOME_URL", f"https://{self.host}"), ("COMMIT", commit), ("SHA256", self.binary_sha)):
            text = text.replace(f"@{name}@", value)
        return text

    async def runner_file(self, request, path):
        if path == "/_home/runner/install.sh":
            status = await self.api("GET", "/api/status")
            return web.Response(text=self.install_script(status.get("commit", "")), content_type="text/x-shellscript",
                                headers={"Cache-Control": "no-store"})
        return web.FileResponse(self.binary, headers={"Cache-Control": "no-store", "Content-Type": "application/octet-stream",
                                                      "Content-Disposition": "attachment; filename=launchpad"})

    # ------------------------------------------------------------------ routes

    def page(self):
        return web.Response(text=Path(__file__).with_name("setup.html").read_text(), content_type="text/html",
                            headers={"Cache-Control": "no-store", "Referrer-Policy": "no-referrer"})

    async def handle(self, request, path):
        """A response for setup routes and while setup is unfinished; None lets the request through."""
        if path == "/_home/status" and request.method == "GET":
            try:
                await self.check_home()
            except Exception:
                pass  # The page shows the last known state.
            return web.json_response(self.status(), headers={"Cache-Control": "no-store"})
        if path in ("/_home/runner/install.sh", "/_home/runner/launchpad") and request.method in ("GET", "HEAD"):
            if not self.unlocked or not self.api:
                return web.Response(status=503, text="echo 'Finish Home setup first.' >&2; exit 1\n")
            return await self.runner_file(request, path)
        if path.startswith("/_home/") and request.method == "POST":
            body = await request.json()
            if not isinstance(body, dict) or not hmac.compare_digest(str(body.get("csrf", "")).encode(), self.csrf.encode()):
                raise web.HTTPForbidden(text="Reload the setup page")
            try:
                extra = await self.action(path, body)
            except ValueError as error:
                return web.json_response({"error": str(error)}, status=409)
            except aiohttp.ClientResponseError:
                return web.json_response({"error": "Home did not accept the change. Try again."}, status=502)
            return web.json_response(dict(self.status(), **(extra or {})), headers={"Cache-Control": "no-store"})
        page = not path.startswith("/api/") and path != "/ws"
        if path == "/_home/setup" or (page and not self.complete):
            return self.page()
        if not self.unlocked:
            return web.json_response({"error": "Finish Home setup first.", "setup": "/_home/setup"}, status=503)
        return None
