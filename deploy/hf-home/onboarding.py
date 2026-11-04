"""Home's first-start setup: Hugging Face (required), GitHub and a first profile.

Only fixed CLI device logins run, and only after an owner click. Their output stays in memory and is never logged.
"""
import asyncio
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal

import aiohttp
from aiohttp import web

ANSI = re.compile(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])")
URL = re.compile(r"https://[^\s<>\"'\x1b\x07]+")
# One-time device codes printed by hf ("ABCD-EFGH"), gh ("ABCD-1234") and Codex ("ABCD-12345").
CODE = re.compile(r"\b[A-Z0-9]{4}-[A-Z0-9]{4,5}\b")
TIMEOUT = 900


def device_prompt(output):
    """The last sign-in link and one-time code printed by a device login."""
    text = ANSI.sub("", output).replace("\r", "")
    links = [u.rstrip(".,;:)") for u in URL.findall(text)]
    codes = CODE.findall(text)
    return (links[-1] if links else None), (codes[-1] if codes else None)


def unique_id(base, taken):
    value, n = base, 2
    while value in taken:
        value, n = f"{base}-{n}", n + 1
    return value


class DeviceLogin:
    """One fixed device-login process; `done` runs after a successful exit."""

    def __init__(self, tool, command, env, done, cleanup=None):
        self.tool, self.command, self.env, self.done, self.cleanup = tool, command, env, done, cleanup
        self.process = None
        self.task = None
        self.output = b""
        self.error = None
        self.cancelled = False

    @property
    def running(self):
        return bool(self.task and not self.task.done())

    async def start(self):
        self.output, self.error, self.cancelled = b"", None, False
        self.process = await asyncio.create_subprocess_exec(
            *self.command, env=self.env, stdin=asyncio.subprocess.DEVNULL, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT, start_new_session=True)
        self.task = asyncio.create_task(self.read())

    async def read(self):
        async def pump():
            while data := await self.process.stdout.read(4096):
                self.output = (self.output + data)[-65536:]
            return await self.process.wait()

        try:
            code = await asyncio.wait_for(pump(), TIMEOUT)
            if code == 0:
                await self.done()
            elif not self.cancelled:
                self.error = "The sign-in did not complete. Start it again."
        except asyncio.TimeoutError:
            await self.stop()
            self.error = "The sign-in code expired. Start it again."
        except Exception as error:  # Never log the output: it can hold codes.
            await self.stop()
            self.error = str(error) or "The sign-in could not be saved."
        finally:
            self.output = b""  # Codes are useless after the process ends.
            if self.cleanup:
                self.cleanup()

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
        return {"tool": self.tool, "running": self.running, "link": link, "code": code, "error": self.error}


class Setup:
    def __init__(self, bucket, credentials):
        self.bucket = bucket
        self.csrf = secrets.token_hex(32)
        self.file = credentials / "setup.json"
        self.gh_dir = credentials / "gh"
        self.gh_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        try:
            self.saved = json.loads(self.file.read_text())
        except (OSError, ValueError):
            self.saved = {}
        self.hf = {"done": False, "account": None, "error": None}
        self.github = {"done": False, "account": None, "error": None}
        self.profiles = []
        self.login = None
        self.lock = asyncio.Lock()
        self.api = None  # Set once the daemon is up: async (method, path, body=None) -> JSON.

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

    async def check_github(self):
        env = dict(os.environ, GH_CONFIG_DIR=str(self.gh_dir), GH_PROMPT_DISABLED="1")
        try:
            process = await asyncio.create_subprocess_exec(
                "gh", "api", "user", "--jq", ".login", env=env, stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL)
            out, _ = await asyncio.wait_for(process.communicate(), 30)
        except (OSError, asyncio.TimeoutError):
            self.github = {"done": False, "account": None, "error": "Could not check the GitHub login."}
            return
        account = out.decode().strip() if process.returncode == 0 else None
        self.github = {"done": bool(account), "account": account or None, "error": None}

    async def check_profiles(self):
        if self.api:
            config = (await self.api("GET", "/api/config"))["config"]
            self.profiles = [p["name"] for p in config["profiles"]]

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

    def step_done(self, step):
        if step == "github":
            return self.github["done"] or self.saved.get("github") == "skipped"
        return bool(self.profiles) or self.saved.get("profile") == "skipped"

    @property
    def complete(self):
        """HF is always required; the other steps only gate the first start."""
        return self.hf["done"] and (self.saved.get("completed") or (self.step_done("github") and self.step_done("profile")))

    def save(self, **values):
        self.saved.update(values)
        temporary = self.file.with_suffix(".tmp")
        temporary.write_text(json.dumps(self.saved))
        temporary.replace(self.file)

    def status(self):
        return {
            "csrf": self.csrf, "bucket": self.bucket, "complete": bool(self.complete),
            "huggingface": self.hf,
            "github": dict(self.github, secret=self.secret(), skipped=self.saved.get("github") == "skipped"),
            "profile": {"done": bool(self.profiles), "profiles": self.profiles, "skipped": self.saved.get("profile") == "skipped",
                        "ready": self.api is not None},
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
                self.login = DeviceLogin(tool, ("hf", "auth", "login", "--force", "--format", "agent"), env, self.check_hf)
            elif tool == "github":
                if not self.hf["done"]:
                    raise ValueError("Sign in to Hugging Face first")
                if self.secret():
                    raise ValueError("GitHub uses the GH_TOKEN Space secret; remove it in the Space settings to sign in here")
                env = {k: v for k, v in base.items() if k not in ("GH_TOKEN", "GITHUB_TOKEN")}
                env.update(GH_CONFIG_DIR=str(self.gh_dir), GH_PROMPT_DISABLED="1")
                command = ("gh", "auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web",
                           "--skip-ssh-key", "--insecure-storage", "--scopes", "repo,read:org,notifications,workflow")
                self.login = DeviceLogin(tool, command, env, self.github_done)
            elif tool == "codex":
                if not self.api:
                    raise ValueError("Home is still starting. Try again in a moment.")
                folder = Path.home() / ".local/state/home-setup" / secrets.token_hex(8)
                folder.mkdir(mode=0o700, parents=True)
                (folder / "config.toml").write_text('cli_auth_credentials_store = "file"\n')
                env = dict(base, CODEX_HOME=str(folder))

                async def done():
                    await self.add_login("codex", "codex_login", "Codex (ChatGPT)", (folder / "auth.json").read_text())

                self.login = DeviceLogin(tool, ("codex", "login", "--device-auth"), env, done,
                                         cleanup=lambda: shutil.rmtree(folder, ignore_errors=True))
            else:
                raise ValueError("Choose a supported sign-in")
            await self.login.start()

    async def github_done(self):
        await self.check_github()
        if self.github["done"] and self.api:
            try:
                await self.api("POST", "/api/inbox/sync", {})
            except Exception:
                pass  # The next poll syncs the inbox.

    async def add_login(self, harness, kind, name, secret):
        """Store a harness login in Home's vault and create a profile that uses it."""
        if not self.api:
            raise ValueError("Home is still starting. Try again in a moment.")
        vault = await self.api("GET", "/api/vault")
        entry = unique_id(harness, {e["id"] for e in vault["entries"]})
        await self.api("PUT", f"/api/vault/entries/{entry}", {"kind": kind, "name": name, "value": secret})
        config = (await self.api("GET", "/api/config"))["config"]
        harness_name = {"claude": "Claude Code", "codex": "Codex"}[harness]
        config["profiles"].append({"id": unique_id(harness, {p["id"] for p in config["profiles"]}),
                                   "name": harness_name, "harness": harness, "login": entry})
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
        elif path == "/_home/check":
            await self.check()
        elif path == "/_home/skip":
            if body.get("step") not in ("github", "profile"):
                raise ValueError("Choose a step")
            self.save(**{body["step"]: "skipped"})
        elif path == "/_home/claude":
            token = str(body.get("token", "")).strip()
            if not token:
                raise ValueError("Paste the token printed by claude setup-token")
            await self.add_login("claude", "claude_login", "Claude Code", token)
        elif path == "/_home/finish":
            if not self.complete:
                raise ValueError("Finish the setup steps first")
        else:
            raise web.HTTPNotFound()
        if self.complete and not self.saved.get("completed"):
            self.save(completed=True)

    # ------------------------------------------------------------------ routes

    def page(self):
        return web.Response(text=Path(__file__).with_name("setup.html").read_text(), content_type="text/html",
                            headers={"Cache-Control": "no-store", "Referrer-Policy": "no-referrer"})

    async def handle(self, request, path):
        """A response for setup routes and while setup is unfinished; None lets the request through."""
        if path == "/_home/status" and request.method == "GET":
            return web.json_response(self.status(), headers={"Cache-Control": "no-store"})
        if path.startswith("/_home/") and path != "/_home/setup" and request.method == "POST":
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
        if not self.hf["done"]:
            return web.json_response({"error": "Hugging Face sign-in is required. Open Home setup.", "setup": "/_home/setup"}, status=503)
        return None
