"""The helper of Launchpad oracles: run(check), Context, verdict(...) and failed(reason).

An oracle file ends with ``run(check)``. Launchpad sends the evaluation's input as JSON on stdin; ``run`` calls
``check(ctx)`` and writes the result as one JSON line at the end of stdout (``print`` inside ``check`` goes to
stderr). ``ctx.http`` is a plain ``httpx2.Client`` with the oracle's timeout that refuses hosts the oracle does not
allow. This API stays backward compatible across Launchpad updates: add, never remove or rename.
"""

from __future__ import annotations

import datetime as _dt
import json
import sys
import traceback
from typing import Any, Callable, NoReturn

import httpx2

__all__ = ["run", "Context", "Params", "Verdict", "verdict", "failed", "Failed", "HostRefused", "MAX_CURSOR", "MAX_EVIDENCE"]
__version__ = "1.0.0"

#: Largest proposed cursor and evidence (summary, fingerprint, links, items), as JSON.
MAX_CURSOR = 4 * 1024
MAX_EVIDENCE = 32 * 1024

# Tells Launchpad that uv prepared the environment: the oracle's timeout counts from here.
sys.stderr.write("\x1elaunchpad_oracle:start\n")
sys.stderr.flush()


class Failed(Exception):
    """Raised by failed(): the oracle cannot decide (Failed, class declared)."""


class HostRefused(Exception):
    """ctx.http refused a host the oracle does not allow (Failed, class access)."""


class _Contract(Exception):
    pass


class Params(dict):
    """The oracle's parameters, typed and validated by Launchpad: ``ctx.params.repo`` or ``ctx.params["repo"]``."""

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError:
            raise AttributeError(f"no parameter {name!r}") from None


_UNSET: Any = object()


class Verdict:
    """The answer of a check: build it with verdict()."""

    __slots__ = ("value", "summary", "fingerprint", "links", "items", "cursor")

    def __init__(self, value: bool, summary: str, fingerprint: str | None, links: list, items: list, cursor: Any):
        self.value, self.summary, self.fingerprint, self.links, self.items, self.cursor = value, summary, fingerprint, links, items, cursor

    def as_json(self) -> dict:
        out = {"outcome": "true" if self.value else "false", "summary": self.summary, "fingerprint": self.fingerprint, "links": self.links, "items": self.items}
        if self.cursor is not _UNSET:
            out["cursor"] = self.cursor
        return out


def _size(value: Any, what: str) -> int:
    try:
        return len(json.dumps(value, ensure_ascii=False).encode())
    except (TypeError, ValueError) as e:
        raise _Contract(f"{what} is not JSON: {e}") from None


def verdict(value: bool, *, summary: str = "", fingerprint: Any = None, links: Any = (), items: Any = (), cursor: Any = _UNSET) -> Verdict:
    """True or False, with its evidence. ``fingerprint`` identifies what is true (a commit, a version) for "fire on
    change"; ``cursor`` (JSON, at most 4 KB) is proposed to the consumer, which commits it when it acts; leave it
    out to keep the current cursor. Evidence is at most 32 KB."""
    if not isinstance(value, bool):
        raise _Contract(f"verdict() takes True or False, not {type(value).__name__}")
    v = Verdict(value, str(summary), None if fingerprint is None else str(fingerprint), [str(x) for x in links], list(items), cursor)
    if cursor is not _UNSET and _size(cursor, "the cursor") > MAX_CURSOR:
        raise _Contract(f"the cursor is larger than {MAX_CURSOR} bytes")
    if _size([v.summary, v.fingerprint, v.links, v.items], "the evidence") > MAX_EVIDENCE:
        raise _Contract(f"the evidence (summary, fingerprint, links, items) is larger than {MAX_EVIDENCE} bytes")
    return v


def failed(reason: str) -> NoReturn:
    """Stops the check with Failed: it cannot decide (a partial page, an unexpected schema). Never return False
    for that."""
    raise Failed(str(reason))


def _allowed(host: str, hosts: list[str]) -> bool:
    host = host.lower().rstrip(".")
    for h in hosts:
        h = h.lower()
        if h == "*" or h == host or (h.startswith("*.") and host.endswith(h[1:])):
            return True
    return False


def _origin(url: Any) -> tuple:
    return (url.scheme, (url.host or "").lower(), url.port)


class Context:
    """What a check gets: params, event, cursor, now, http (and launchpad with Launchpad read access)."""

    def __init__(self, data: dict):
        self.params = Params(data.get("params") or {})
        #: Why it runs: {"kind": "schedule" | "cli" | "test" | ..., ...}.
        self.event: dict = data.get("event") or {}
        #: The consumer's committed cursor, or None.
        self.cursor: Any = data.get("cursor")
        #: The evaluation's start time (UTC), fixed for the whole run.
        now = data.get("now")
        self.now = _dt.datetime.fromtimestamp(now / 1000, _dt.timezone.utc) if isinstance(now, (int, float)) else _dt.datetime.now(_dt.timezone.utc)
        #: The oracle's timeout in seconds.
        self.timeout: float = float(data.get("timeout") or 10)
        #: Hosts ctx.http may call ("*" = all, "*.example.com" = its subdomains).
        self.hosts: list[str] = list(data.get("hosts") or [])
        #: With launchpad = "read": {"url", "token", "workspace"} (also LAUNCHPAD_URL, LAUNCHPAD_TOKEN and
        #: LAUNCHPAD_WORKSPACE): Home's read API for this workspace, through ctx.http with
        #: ``Authorization: Bearer <token>``; else None.
        self.launchpad: dict | None = data.get("launchpad")
        self.refused: list[str] = []
        self._home = _origin(httpx2.URL(self.launchpad["url"])) if self.launchpad and self.launchpad.get("url") else None
        self.http = httpx2.Client(timeout=self.timeout, event_hooks={"request": [self._check_host]})

    def _check_host(self, request: Any) -> None:
        if self._home is not None and _origin(request.url) == self._home:
            return
        host = request.url.host or ""
        if not _allowed(host, self.hosts):
            self.refused.append(host)
            raise HostRefused(f"{host} is not an allowed host of this oracle (allowed: {', '.join(self.hosts) or 'none'}): add it to access.hosts")


def _fail(cls: str, message: str) -> dict:
    return {"outcome": "failed", "error": {"class": cls, "message": message[:2000]}}


def _http_message(e: Exception) -> str:
    if isinstance(e, httpx2.HTTPStatusError):
        r = e.response
        return f"{r.status_code} {r.reason_phrase} from {r.request.url.host} ({r.request.method} {r.request.url.copy_with(query=None)})"
    return f"{type(e).__name__}: {e}"


def run(check: Callable[[Context], Verdict]) -> None:
    """Reads the input, calls check(ctx) and writes the result. An uncaught exception becomes Failed."""
    out = sys.stdout
    sys.stdout = sys.stderr
    try:
        raw = sys.stdin.read()
        ctx = Context(json.loads(raw) if raw.strip() else {})
    except Exception as e:  # noqa: BLE001
        result = _fail("contract", f"unreadable input: {e}")
    else:
        try:
            try:
                res = check(ctx)
            finally:
                ctx.http.close()
            if not isinstance(res, Verdict):
                raise _Contract(f"check() returned {type(res).__name__}: return verdict(True or False, ...) or call failed(reason)")
            result = res.as_json()
        except Failed as e:
            result = _fail("declared", str(e) or "failed")
        except HostRefused as e:
            result = _fail("access", str(e))
        except _Contract as e:
            result = _fail("contract", str(e))
        except (httpx2.HTTPError, httpx2.InvalidURL) as e:
            result = _fail("access" if ctx.refused else "http", _http_message(e))
        except Exception as e:  # noqa: BLE001
            traceback.print_exc()
            if ctx.refused:
                result = _fail("access", f"{ctx.refused[-1]} is not an allowed host of this oracle ({type(e).__name__}: {e})")
            else:
                result = _fail("script", f"{type(e).__name__}: {e}")
    sys.stdout = out
    out.write("\n" + json.dumps(result, ensure_ascii=False) + "\n")
    out.flush()
