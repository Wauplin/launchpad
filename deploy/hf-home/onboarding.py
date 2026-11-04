"""Home's first-start setup: Hugging Face with the credential key, GitHub and a first profile. All three are required.

Only fixed CLI logins run, and only after an owner click. Their output stays in memory and is never logged.
Credentials are encrypted with `LP_HOME_KEY` (a Space secret): Home's own logins in the bucket (decrypted only
into this container's local disk) and the vault secrets in Home's backups. Everything else is stored plain.
"""
import asyncio
import base64
import errno
import hmac
import os
from pathlib import Path
import pty
import re
import secrets
import shutil
import signal

import aiohttp
from aiohttp import web
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

ANSI = re.compile(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_]|[78])")
# OSC 8 hyperlinks carry the whole URL even when the visible text is cut at the terminal width.
OSC8 = re.compile(r"\x1b\]8;[^;\x07\x1b]*;(https://[^\x07\x1b]+)(?:\x07|\x1b\\)")
URL = re.compile(r"https://[^\s<>\"'\x1b\x07]+")
# One-time device codes printed by hf ("ABCD-EFGH"), gh ("ABCD-1234") and Codex ("ABCD-12345").
CODE = re.compile(r"\b[A-Z0-9]{4}-[A-Z0-9]{4,5}\b")
CLAUDE_TOKEN = re.compile(r"sk-ant-oat01-[A-Za-z0-9_-]{20,}")
# Claude Code's answer to a rejected code; cursor moves may replace the spaces.
RETRY = re.compile(r"Press\s*Enter\s*to\s*retry")
TIMEOUT = 900
MAGIC = b"LPSEALED1"
KEY_SECRET = "LP_HOME_KEY"
LOGINS = {"claude": "claude_login", "codex": "codex_login"}  # Harnesses whose login Home keeps in its vault.
NAMES = {"claude": "Claude Code", "codex": "Codex"}


def device_prompt(output):
    """The last sign-in link and one-time code printed by a login."""
    links = OSC8.findall(output)
    text = ANSI.sub("", output).replace("\r", "")
    links = links or [u.rstrip(".,;:)") for u in URL.findall(text)]
    codes = CODE.findall(text)
    return (links[-1] if links else None), (codes[-1] if codes else None)


def claude_token(output):
    found = CLAUDE_TOKEN.findall(ANSI.sub("", output))
    return found[-1] if found else None


def unique_id(base, taken):
    value, n = base, 2
    while value in taken:
        value, n = f"{base}-{n}", n + 1
    return value


def parse_key(value):
    try:
        key = base64.b64decode(value.strip(), validate=True)
    except ValueError:
        return None
    return key if len(key) == 32 else None


def seal(key, data):
    """Same format as Home's backups: MAGIC || nonce || AES-256-GCM ciphertext and tag."""
    nonce = os.urandom(12)
    return MAGIC + nonce + AESGCM(key).encrypt(nonce, data, None)


def unseal(key, data):
    if not data.startswith(MAGIC):
        return data  # From before encryption.
    if key is None:
        raise ValueError("This object is encrypted: the LP_HOME_KEY Space secret is missing")
    rest = data[len(MAGIC):]
    return AESGCM(key).decrypt(rest[:12], rest[12:], None)


def write_private(path, data):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(data)
    temporary.chmod(0o600)
    temporary.replace(path)


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
            elif code is not None and not self.cancelled:
                self.error = "The sign-in did not complete. Start it again."
        except asyncio.TimeoutError:
            await self.stop()
            self.error = "The sign-in expired. Start it again."
        except Exception as error:  # Never log the output: it can hold codes and tokens.
            await self.stop()
            self.error = str(error) or "The sign-in could not be saved."
        finally:
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
    def __init__(self, space, bucket, credentials, key):
        self.space, self.bucket = space, bucket
        self.key = key  # From the LP_HOME_KEY Space secret; None until encryption is set up.
        self.csrf = secrets.token_hex(32)
        self.sealed = credentials / "sealed"
        self.legacy = credentials
        self.hf_home = Path(os.environ["HF_HOME"])
        self.gh_dir = Path(os.environ["GH_CONFIG_DIR"])
        self.files = {"huggingface": (self.hf_home / "token", self.sealed / "huggingface-token"),
                      "github": (self.gh_dir / "hosts.yml", self.sealed / "gh-hosts.yml")}
        self.hf = {"done": False, "account": None, "error": None}
        self.github = {"done": False, "account": None, "error": None}
        self.profile = {"done": False, "profiles": [], "missing": None, "logins": []}
        self.encryption = {"state": "on" if key else "off", "error": None, "manual_key": None}
        self.login = None
        self.lock = asyncio.Lock()
        self.api = None  # Set once the daemon is up: async (method, path, body=None) -> JSON.

    # ------------------------------------------------------------------ credentials at rest

    def restore(self):
        """Decrypt Home's logins from the bucket; encrypt and remove plain-text copies from before encryption."""
        legacy = {"huggingface": self.legacy / "huggingface/token", "github": self.legacy / "gh/hosts.yml"}
        for name, (local, sealed) in self.files.items():
            if sealed.exists() and self.key:
                write_private(local, unseal(self.key, sealed.read_bytes()))
            elif legacy[name].exists() and not local.exists():
                write_private(local, legacy[name].read_bytes())
            if self.key and local.exists():
                self.save(name)
        if self.key:
            for folder in ("huggingface", "gh"):
                shutil.rmtree(self.legacy / folder, ignore_errors=True)

    def save(self, name, key=None):
        key = key or self.key
        local, sealed = self.files[name]
        if key and local.exists():
            write_private(sealed, seal(key, local.read_bytes()))

    async def enable_encryption(self):
        """After the HF sign-in: create the key, keep it as a Space secret and restart Home once to use it."""
        if self.key or self.encryption["state"] in ("restarting", "manual"):
            return
        key = os.urandom(32)
        value = base64.b64encode(key).decode()
        self.save("huggingface", key)
        try:
            from huggingface_hub import HfApi, get_token
            api = HfApi(token=get_token())
            await asyncio.to_thread(api.add_space_secret, self.space, KEY_SECRET, value,
                                    description="Encrypts Launchpad Home's credentials in the bucket. Never share or delete it.")
            self.encryption = {"state": "restarting", "error": None, "manual_key": None}
            asyncio.get_running_loop().call_later(2, lambda: asyncio.ensure_future(self.restart(api)))
        except Exception:
            self.encryption = {"state": "manual", "manual_key": value,
                               "error": "This Hugging Face login cannot add Space secrets. Add the secret below in the Space settings, then restart the Space."}

    async def restart(self, api):
        try:
            await asyncio.to_thread(api.restart_space, self.space)
        except Exception:
            pass  # Adding a secret restarts the Space on its own.

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
            # A stored token is enough (no network); the account name is best effort.
            stored = self.secret() or (self.gh_dir / "hosts.yml").exists()
            if not stored or not await self.gh("auth", "token", "--hostname", "github.com"):
                self.github = {"done": False, "account": None, "error": None}
                return
            self.github = {"done": True, "account": await self.gh("api", "user", "--jq", ".login"), "error": None}
        except (OSError, asyncio.TimeoutError):
            self.github = {"done": False, "account": None, "error": "Could not check the GitHub login."}

    async def check_profiles(self):
        """Done when a profile can run: its harness login is in the vault (or it needs no vault login)."""
        if not self.api:
            return
        config = (await self.api("GET", "/api/config"))["config"]
        entries = (await self.api("GET", "/api/vault"))["entries"]
        ready, missing = [], None
        for p in config["profiles"]:
            kind = LOGINS.get(p["harness"])
            ok = kind is None or any(e["kind"] == kind and (not p.get("login") or e["id"] == p["login"]) for e in entries)
            if ok:
                ready.append(p["name"])
            elif missing is None:
                missing = {"name": p["name"], "harness": p["harness"]}
        logins = sorted({h for h, kind in LOGINS.items() if any(e["kind"] == kind for e in entries)})
        self.profile = {"done": bool(ready), "profiles": ready, "missing": missing, "logins": logins}

    async def check(self):
        await self.check_hf()
        await self.check_github()
        try:
            await self.check_profiles()
        except Exception:
            pass

    # ------------------------------------------------------------------ state

    def secret(self):
        return bool(os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN"))

    @property
    def unlocked(self):
        """Home's data may be used: signed in to HF with encryption on."""
        return self.hf["done"] and self.key is not None

    @property
    def complete(self):
        return self.unlocked and self.github["done"] and self.profile["done"]

    def status(self):
        return {
            "csrf": self.csrf, "bucket": self.bucket, "space": self.space, "complete": self.complete,
            "huggingface": dict(self.hf, done=self.unlocked, signed_in=self.hf["done"]),
            "encryption": self.encryption,
            "github": dict(self.github, secret=self.secret()),
            "profile": dict(self.profile, ready=self.api is not None),
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
                raise ValueError("Finish the Hugging Face step first")
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
                        token = claude_token(output)
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
        if self.hf["done"]:
            if self.key:
                self.save("huggingface")
            else:
                await self.enable_encryption()

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
        await self.check_profiles()

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
        elif path == "/_home/finish":
            if not self.complete:
                raise ValueError("Finish the setup steps first")
        else:
            raise web.HTTPNotFound()

    # ------------------------------------------------------------------ routes

    def page(self):
        return web.Response(text=Path(__file__).with_name("setup.html").read_text(), content_type="text/html",
                            headers={"Cache-Control": "no-store", "Referrer-Policy": "no-referrer"})

    async def handle(self, request, path):
        """A response for setup routes and while setup is unfinished; None lets the request through."""
        if path == "/_home/status" and request.method == "GET":
            return web.json_response(self.status(), headers={"Cache-Control": "no-store"})
        if path.startswith("/_home/") and request.method == "POST":
            body = await request.json()
            if not isinstance(body, dict) or not hmac.compare_digest(str(body.get("csrf", "")).encode(), self.csrf.encode()):
                raise web.HTTPForbidden(text="Reload the setup page")
            try:
                await self.action(path, body)
            except ValueError as error:
                return web.json_response({"error": str(error)}, status=409)
            except aiohttp.ClientResponseError:
                return web.json_response({"error": "Home did not accept the change. Check Settings in Home."}, status=502)
            return web.json_response(self.status(), headers={"Cache-Control": "no-store"})
        page = not path.startswith("/api/") and path != "/ws"
        if path == "/_home/setup" or (page and not self.complete):
            return self.page()
        if not self.unlocked:
            return web.json_response({"error": "Finish Home setup first.", "setup": "/_home/setup"}, status=503)
        return None
