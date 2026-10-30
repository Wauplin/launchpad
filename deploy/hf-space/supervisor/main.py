"""Minimal locked front door. Production admission remains gated on platform proof."""
import asyncio
import os
from pathlib import Path

from aiohttp import web

from gateway.proxy import Proxy
from supervisor.policy import Deployment, verify

SHELL = Path(__file__).resolve().parents[1] / "gateway"


async def serve():
    deployment = Deployment(os.environ["SPACE_ID"], os.environ["LP_SPACE_BUCKET"],
                            "https://" + os.environ["SPACE_HOST"])
    state = {"ready": False, "settings": f"https://huggingface.co/spaces/{deployment.space}/settings",
             "message": "Checking Space and bucket privacy."}
    try:
        await verify(deployment, os.environ["LP_SPACE_MANAGEMENT_TOKEN"])
        state["message"] = "Privacy verified. Runtime admission is pending the documented isolation and recovery gates."
    except ValueError as error:
        state["message"] = str(error)
    except Exception:
        state["message"] = "Could not verify startup. Check Space Secrets, the backup bucket and HF access, then restart."

    # Do not launch a daemon until the P0 / recovery acceptance gates are met.
    proxy = Proxy(deployment.origin, "")
    @web.middleware
    async def boundaries(request, handler):
        proxy.guard(request)
        response = await handler(request)
        response.headers["Cache-Control"] = "no-store"
        response.headers["X-Content-Type-Options"] = "nosniff"
        response.headers["Referrer-Policy"] = "no-referrer"
        response.headers["Content-Security-Policy"] = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-src 'self'; frame-ancestors 'self' https://huggingface.co; object-src 'none'; base-uri 'none'"
        return response

    async def status(request):
        return web.json_response(state)

    async def shell(request):
        return web.FileResponse(SHELL / "shell.html")

    async def script(request):
        return web.FileResponse(SHELL / "shell.js")

    app = web.Application(middlewares=[boundaries], client_max_size=1024 * 1024)
    app.router.add_get("/_space/status", status)
    app.router.add_get("/_space/shell.js", script)
    app.router.add_get("/{path:.*}", shell)
    runner = web.AppRunner(app, access_log=None)
    await runner.setup()
    await web.TCPSite(runner, "0.0.0.0", 7860).start()
    try:
        await asyncio.Event().wait()
    finally:
        await runner.cleanup()


if __name__ == "__main__":
    try:
        asyncio.run(serve())
    except KeyboardInterrupt:
        pass
