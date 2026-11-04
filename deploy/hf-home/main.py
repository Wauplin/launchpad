"""Private Home-only Space. The Hub authenticates visitors; runners keep their own credential."""
import asyncio
import hashlib
import json
import os
from pathlib import Path
import re
import signal

import aiohttp
from aiohttp import web
import tomlkit

from gateway.proxy import Proxy, canonical_path
from onboarding import Setup, parse_key, unseal


def validate_config(method, value):
    if not isinstance(value, dict):
        raise ValueError("Expected a configuration object")
    if method == "PUT" and value.get("home", {}).get("backup") != "/backups":
        raise ValueError("Home backup is required and its bucket mount is managed by the deployment")
    if any(k.startswith("trace_backup_") for k in value) or value.get("trace_backup", {}).get("enabled"):
        raise ValueError("Home backup is always on; legacy trace sync settings do not apply")


class HomeProxy(Proxy):
    def __init__(self, origin, token, setup, port=7319):
        super().__init__(origin, token, port)
        self.setup = setup

    async def handle(self, request):
        path = self.guard(request)
        response = await self.setup.handle(request, path)
        if response is not None:
            return response
        body = None
        if path == "/api/config" and request.method in ("PUT", "PATCH"):
            body = await request.read()
            try:
                validate_config(request.method, json.loads(body))
            except (ValueError, AttributeError):
                return web.json_response({"error": "Home backup is required and managed by the deployment; legacy trace sync settings do not apply"}, status=400)
        return await super().handle(request, body=body)

    def guard(self, request):
        path = canonical_path(request.raw_path)
        machine = (path in ("/api/runners/request", "/api/runners/tunnel")
                   or re.fullmatch(r"/api/runners/request/[^/]+", path)
                   or path.startswith("/api/git/")
                   or (path.startswith("/api/plain/") and "X-Launchpad-Runner" in request.headers))
        # Native clients have no browser Origin. Browser requests still use the normal CSRF checks.
        if machine and "Origin" not in request.headers and "Sec-Fetch-Site" not in request.headers:
            headers = request.headers.copy()
            headers["Origin"] = self.origin
            return super().guard(request.clone(headers=headers))
        return super().guard(request)


async def verify_private(space):
    # No management credential on Home. Public/protected metadata is readable anonymously; private is not.
    async with aiohttp.ClientSession(trust_env=False) as client:
        async with client.get(f"https://huggingface.co/api/spaces/{space}", timeout=aiohttp.ClientTimeout(total=30)) as r:
            if r.status not in (401, 404):
                raise RuntimeError("Home requires a private Space: anonymous Hub metadata must be denied")


def configure(backup, config, key):
    if not backup.is_mount() or not os.access(backup, os.W_OK):
        raise RuntimeError("Mount the private backup bucket read-write at /backups before starting Home")
    config.parent.mkdir(parents=True, exist_ok=True)
    # Recover settings before adding the fixed front door. The daemon restores state.db itself.
    if not config.exists():
        # Encrypted backups live under home/sealed; plain-text ones from before encryption under home.
        roots = ([backup / "home/sealed"] if key else []) + [backup / "home"]
        root = next((r for r in roots if any((r / "backup").glob("*/manifest.json"))), None)
        manifests = sorted((root / "backup").glob("*/manifest.json"), reverse=True) if root else []
        if manifests:
            manifest = json.loads(unseal(key, manifests[0].read_bytes()))
            obj = next(o for o in manifest["objects"] if o["name"] == "config.toml")
            data = unseal(key, (root / "blobs/sha256" / obj["sha256"]).read_bytes())
            if hashlib.sha256(data).hexdigest() != obj["sha256"]:
                raise RuntimeError("Backup config failed its checksum")
            config.write_bytes(data)
    document = tomlkit.parse(config.read_text() if config.exists() else "")
    document.setdefault("server", {}).update(bind="127.0.0.1", public_origin="https://" + os.environ["SPACE_HOST"], auto_tailnet=False)
    document.setdefault("home", {})["backup"] = str(backup)
    document.setdefault("trace_backup", {})["enabled"] = False
    config.write_text(tomlkit.dumps(document))
    config.chmod(0o600)


async def main():
    space = os.environ["SPACE_ID"]
    host = os.environ["SPACE_HOST"]
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", space) or not re.fullmatch(r"[a-z0-9-]+\.hf\.space", host):
        raise RuntimeError("Expected an HF Space ID and exact hf.space host")
    await verify_private(space)
    backup = Path("/backups")
    home = Path.home()
    secret = os.environ.pop("LP_HOME_KEY", "").strip()
    key = parse_key(secret) if secret else None
    if secret and key is None:
        raise RuntimeError("The LP_HOME_KEY Space secret must be base64 of 32 bytes. Restore its original value.")
    # Home's own HF and GitHub logins: decrypted on this container's disk, encrypted in the bucket.
    os.environ["HF_HOME"] = str(home / ".cache/huggingface")
    os.environ["GH_CONFIG_DIR"] = str(home / ".config/gh")
    configure(backup, home / ".config/launchpad/config.toml", key)
    setup = Setup(space, os.environ.get("LP_HOME_BUCKET", space + "-bucket"), backup / "home/credentials", key)
    setup.restore()
    await setup.check()
    env = dict(os.environ, LAUNCHPAD_HOME_BACKUP=str(backup), LAUNCHPAD_PORT="7319", LAUNCHPAD_DELEGATED_ACCESS="true")
    if key:
        env["LAUNCHPAD_HOME_KEY"] = secret
    daemon = await asyncio.create_subprocess_exec("launchpad", "daemon", "--role", "home", env=env)
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    runner = api_client = None
    try:
        token_path = home / ".local/share/launchpad/token"
        async with aiohttp.ClientSession() as client:
            for _ in range(600):
                if daemon.returncode is not None:
                    raise RuntimeError(f"Home exited with {daemon.returncode}")
                try:
                    async with client.get("http://127.0.0.1:7319/api/health") as res:
                        if res.status == 200 and token_path.exists():
                            break
                except aiohttp.ClientError:
                    pass
                await asyncio.sleep(1)
            else:
                raise RuntimeError("Home did not start within ten minutes")
        proxy = HomeProxy("https://" + host, token_path.read_text().strip(), setup, port=7319)
        api_client = aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=60))
        api_headers = {"Host": host, "Origin": "https://" + host, "Authorization": f"Bearer {proxy.token}"}

        async def api(method, path, body=None):
            async with api_client.request(method, "http://127.0.0.1:7319" + path, json=body, headers=api_headers) as res:
                text = await res.text()
                data = json.loads(text) if text.strip().startswith(("{", "[")) else {}
                if res.status >= 400:
                    raise ValueError((data.get("error") if isinstance(data, dict) else None) or f"Home answered {res.status}")
                return data

        setup.api = api
        await setup.check_profiles()
        app = web.Application(client_max_size=4 << 30)
        app.on_startup.append(proxy.start)
        app.on_cleanup.append(proxy.close)
        app.router.add_route("*", "/{path:.*}", proxy.handle)
        runner = web.AppRunner(app, access_log=None)
        await runner.setup()
        await web.TCPSite(runner, "0.0.0.0", 7860).start()
        print(f"Home ready: https://{host}", flush=True)
        stopped = asyncio.create_task(stop.wait())
        exited = asyncio.create_task(daemon.wait())
        await asyncio.wait([stopped, exited], return_when=asyncio.FIRST_COMPLETED)
        stopped.cancel()
        await asyncio.gather(stopped, return_exceptions=True)
        if daemon.returncode is not None and not stop.is_set():
            raise RuntimeError(f"Home exited with {daemon.returncode}")
    finally:
        await setup.close()
        if api_client:
            await api_client.close()
        if runner:
            await runner.cleanup()
        if daemon.returncode is None:
            daemon.terminate()
            # Let the daemon save its final backup before the container exits.
            await daemon.wait()


if __name__ == "__main__":
    asyncio.run(main())
