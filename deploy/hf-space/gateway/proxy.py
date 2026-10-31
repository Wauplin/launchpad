"""Bearer-injecting streaming proxy. HF policy stays outside Launchpad."""
import asyncio
from urllib.parse import unquote

from aiohttp import ClientSession, ClientTimeout, WSMsgType, web

HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
       "te", "trailer", "transfer-encoding", "upgrade"}
IDENTITY = {"authorization", "cookie", "host", "forwarded", "origin"}
BLOCKED = ("/auth", "/api/devices", "/api/push", "/api/open", "/api/window", "/api/setup/fix")


def canonical_path(raw):
    path = raw.split("?", 1)[0]
    # Multiple decoding layers, separators and dot segments create route aliases.
    decoded = unquote(path, errors="strict")
    if "%" in decoded or "\\" in decoded or "//" in decoded or any(p in (".", "..") for p in decoded.split("/")):
        raise web.HTTPBadRequest(text="Invalid route")
    return decoded


def denied(path):
    return any(path == p or path.startswith(p + "/") for p in BLOCKED)


class Proxy:
    def __init__(self, origin, token, port=7317):
        self.origin = origin
        self.host = origin.removeprefix("https://")
        self.token = token
        self.base = f"http://127.0.0.1:{port}"
        self.client = None

    async def start(self, app):
        self.client = ClientSession(timeout=ClientTimeout(total=None, sock_connect=10), auto_decompress=False)

    async def close(self, app):
        await self.client.close()

    def guard(self, request):
        if request.headers.getall("Host", []) != [self.host]:
            raise web.HTTPForbidden(text="Unexpected host")
        if request.method not in ("GET", "HEAD") or request.headers.get("Upgrade"):
            if request.headers.getall("Origin", []) != [self.origin]:
                raise web.HTTPForbidden(text="Use the Space origin")
        # The Hub page embeds the Space in a cross-site iframe: allow top-level/iframe page loads only.
        navigation = request.method in ("GET", "HEAD") and request.headers.get("Sec-Fetch-Mode") == "navigate"
        if request.headers.get("Sec-Fetch-Site") == "cross-site" and not navigation:
            raise web.HTTPForbidden(text="Cross-site requests are disabled")
        return canonical_path(request.raw_path)

    def headers(self, request):
        connection = {s.strip().lower() for s in request.headers.get("Connection", "").split(",")}
        headers = {k: v for k, v in request.headers.items()
                   if k.lower() not in HOP | IDENTITY | connection
                   and not k.lower().startswith(("x-forwarded-", "x-hf-", "x-auth-", "sec-websocket-"))}
        headers.update(Host=self.host, Origin=self.origin, Authorization=f"Bearer {self.token}")
        return headers

    async def handle(self, request):
        path = self.guard(request)
        if denied(path) or path.startswith("/_space"):
            raise web.HTTPNotFound()
        headers = self.headers(request)
        url = self.base + request.raw_path
        if request.headers.get("Upgrade", "").lower() == "websocket":
            return await self.socket(request, url, headers)
        assert self.client
        async with self.client.request(request.method, url, headers=headers,
                                       data=request.content.iter_chunked(65536), allow_redirects=False) as upstream:
            clean = {k: v for k, v in upstream.headers.items() if k.lower() not in HOP | {"set-cookie"}}
            clean["Cache-Control"] = "no-store" if path.startswith("/api/") else clean.get("Cache-Control", "no-cache")
            response = web.StreamResponse(status=upstream.status, headers=clean)
            await response.prepare(request)
            async for chunk in upstream.content.iter_chunked(65536):
                await response.write(chunk)
            await response.write_eof()
            return response

    async def socket(self, request, url, headers):
        assert self.client
        async with self.client.ws_connect(url, headers=headers, max_msg_size=16 * 1024 * 1024) as upstream:
            downstream = web.WebSocketResponse(max_msg_size=16 * 1024 * 1024)
            await downstream.prepare(request)

            async def pipe(source, target):
                async for message in source:
                    if message.type == WSMsgType.TEXT:
                        await target.send_str(message.data)
                    elif message.type == WSMsgType.BINARY:
                        await target.send_bytes(message.data)
                    elif message.type in (WSMsgType.CLOSE, WSMsgType.ERROR):
                        break
            tasks = [asyncio.create_task(pipe(upstream, downstream)), asyncio.create_task(pipe(downstream, upstream))]
            try:
                await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
                await downstream.close()
            return downstream
