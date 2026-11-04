"""Owner-clicked HF device login. Only the fixed CLI command runs; output stays in memory."""
import asyncio
import hmac
import os
from pathlib import Path
import re
import secrets
import signal

import aiohttp
from aiohttp import web


class Login:
    def __init__(self, bucket):
        self.bucket = bucket
        self.csrf = secrets.token_hex(32)
        self.signed_in = False
        self.account = None
        self.error = None
        self.process = None
        self.task = None
        self.output = b""
        self.lock = asyncio.Lock()

    async def check(self):
        from huggingface_hub import get_token
        token = get_token()
        if not token:
            self.signed_in = False
            self.error = "Sign in to Hugging Face to finish setting up Home."
            return
        try:
            async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as client:
                headers = {"Authorization": f"Bearer {token}"}
                async with client.get("https://huggingface.co/api/whoami-v2", headers=headers) as response:
                    response.raise_for_status()
                    account = (await response.json())["name"]
                async with client.get(f"https://huggingface.co/api/buckets/{self.bucket}", headers=headers) as response:
                    response.raise_for_status()
                    bucket = await response.json()
                    if bucket.get("private") is not True:
                        raise ValueError("Home's backup bucket must be private")
            self.signed_in, self.account, self.error = True, account, None
        except Exception:
            self.signed_in = False
            self.error = "Could not verify this login and access to the private backup bucket. Check the login and bucket permissions, then check again."

    def status(self):
        text = self.output.decode(errors="replace")
        links = re.findall(r"https://huggingface\.co/[^\s\x1b<>\"']+", text)
        codes = re.findall(r"\b[A-Z0-9]{4}-[A-Z0-9]{4,5}\b", text)
        return {"signed_in": self.signed_in, "account": self.account, "bucket": self.bucket,
                "running": bool(self.process and self.process.returncode is None), "error": self.error,
                "link": links[-1] if links else None, "code": codes[-1] if codes else None, "csrf": self.csrf}

    async def start(self):
        async with self.lock:
            if self.task and not self.task.done():
                return
            self.output, self.error = b"", None
            env = dict(os.environ, HF_HUB_DISABLE_UPDATE_CHECK="1", BROWSER="/bin/true", PYTHONUNBUFFERED="1")
            self.process = await asyncio.create_subprocess_exec(
                "hf", "auth", "login", "--force", "--format", "agent", env=env,
                stdin=asyncio.subprocess.DEVNULL, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT, start_new_session=True)
            self.task = asyncio.create_task(self.read())

    async def read(self):
        try:
            async with asyncio.timeout(900):
                while data := await self.process.stdout.read(4096):
                    self.output = (self.output + data)[-65536:]
                code = await self.process.wait()
                if code == 0:
                    await self.check()
                else:
                    self.error = "Sign-in did not complete. Try again."
        except TimeoutError:
            await self.stop_process()
            self.error = "The sign-in code expired. Try again."

    async def stop_process(self):
        if self.process and self.process.returncode is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
                await asyncio.wait_for(self.process.wait(), 10)
            except ProcessLookupError:
                pass
            except TimeoutError:
                os.killpg(self.process.pid, signal.SIGKILL)
                await self.process.wait()

    async def close(self):
        await self.stop_process()
        if self.task:
            await asyncio.gather(self.task, return_exceptions=True)

    async def handle(self, request, path):
        if path == "/_home/status" and request.method == "GET":
            return web.json_response(self.status(), headers={"Cache-Control": "no-store"})
        if path in ("/_home/login", "/_home/check") and request.method == "POST":
            body = await request.json()
            if not hmac.compare_digest(str(body.get("csrf", "")).encode(), self.csrf.encode()):
                raise web.HTTPForbidden(text="Reload the setup page")
            if path == "/_home/login":
                await self.start()
            else:
                await self.check()
            return web.json_response(self.status(), headers={"Cache-Control": "no-store"})
        if path == "/_home/setup" or (not self.signed_in and not path.startswith("/api/") and path != "/ws"):
            return web.Response(text=Path(__file__).with_name("setup.html").read_text(), content_type="text/html",
                                headers={"Cache-Control": "no-store", "Referrer-Policy": "no-referrer"})
        if not self.signed_in:
            return web.json_response({"error": "Hugging Face sign-in is required. Open Home setup.", "setup": "/_home/setup"}, status=503)
        return None
