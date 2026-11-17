"""launchpad_oracle: everything an oracle imports.

Launchpad serves this module read-only to Monty on Home. Every tool call goes through `launchpad_host.call`, which
Home checks, runs and logs; `failed` and `raise_for_status` end the evaluation through `launchpad_host.fail`, which
the oracle cannot catch. Values cross the boundary as JSON text.
"""

import json
from datetime import datetime, timezone

import launchpad_host


class Response:
    """The answer of a `.get`: the final status, the body as text (None when it is not UTF-8) and its JSON value."""

    def __init__(self, status_code, text, json_value, call):
        self.status_code = status_code
        self.text = text
        self.json = json_value
        self._call = call

    def raise_for_status(self):
        if self.status_code >= 400:
            launchpad_host.fail("http", "HTTP " + str(self.status_code), self._call)
        return self

    def __repr__(self):
        return "<Response [" + str(self.status_code) + "]>"


class Verdict:
    """The answer of check: build it with verdict()."""

    def __init__(self, value, summary, fingerprint, links, items, cursor, cursor_set):
        self.value = value
        self.summary = summary
        self.fingerprint = fingerprint
        self.links = links
        self.items = items
        self.cursor = cursor
        self.cursor_set = cursor_set

    def __repr__(self):
        return "verdict(" + repr(self.value) + ", summary=" + repr(self.summary) + ")"


class _Unset:
    pass


_UNSET = _Unset()


def verdict(value, *, summary="", fingerprint=None, links=None, items=None, cursor=_UNSET):
    """True or False with its evidence; `cursor` proposes the caller's next cursor (leave it out to keep it)."""
    if cursor is _UNSET:
        return Verdict(value, summary, fingerprint, links, items, None, False)
    return Verdict(value, summary, fingerprint, links, items, cursor, True)


def failed(reason):
    """Ends the evaluation with Failed (class declared): the oracle cannot decide."""
    launchpad_host.fail("declared", str(reason), -1)


def _plain(value):
    """A value JSON can carry: datetimes become ISO 8601 in UTC."""
    if isinstance(value, datetime):
        if value.tzinfo is None:
            value = value.replace(tzinfo=timezone.utc)
        return value.astimezone(timezone.utc).isoformat()
    if isinstance(value, dict):
        return {k: _plain(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_plain(v) for v in value]
    return value


def _run(tool, op, args):
    return launchpad_host.call(tool, op, json.dumps(_plain(args)))


def _get(tool, target, params, headers):
    r = _run(tool, "get", {"target": target, "params": params, "headers": headers})
    text = r["text"]
    value = None
    if text:
        try:
            value = json.loads(text)
        except Exception:
            value = None
    return Response(r["status_code"], text, value, r["call"])


def _command(tool, op, args):
    return json.loads(_run(tool, op, args))


class _Http:
    def get(self, url, params=None, headers=None):
        return _get("http", url, params, headers)


class _Api:
    def __init__(self, name):
        self._name = name

    def get(self, endpoint, params=None, headers=None):
        return _get(self._name, endpoint, params, headers)


class _SlackSearch:
    def messages(self, query, limit=None, page=None):
        return _command("slack", "search.messages", {"query": query, "limit": limit, "page": page})


class _SlackMe:
    def channels(self, limit=None, dms=False, unread=False):
        return _command("slack", "me.channels", {"limit": limit, "dms": dms, "unread": unread})


class _SlackChannels:
    def list(self, limit=None):
        return _command("slack", "channels.list", {"limit": limit})

    def info(self, channel):
        return _command("slack", "channels.info", {"channel": channel})

    def history(self, channel, limit=None, after=None, before=None, enrich=False):
        args = {"channel": channel, "limit": limit, "after": after, "before": before, "enrich": enrich}
        return _command("slack", "channels.history", args)

    def members(self, channel, limit=None):
        return _command("slack", "channels.members", {"channel": channel, "limit": limit})

    def pins(self, channel):
        return _command("slack", "channels.pins", {"channel": channel})


class _SlackMessages:
    def replies(self, channel, thread_ts, limit=None):
        return _command("slack", "messages.replies", {"channel": channel, "thread_ts": thread_ts, "limit": limit})

    def permalink(self, channel, ts):
        return _command("slack", "messages.permalink", {"channel": channel, "ts": ts})

    def reactions(self, channel, ts):
        return _command("slack", "messages.reactions", {"channel": channel, "ts": ts})


class _SlackDms:
    def list(self, limit=None):
        return _command("slack", "dms.list", {"limit": limit})

    def history(self, dm_channel, limit=None, after=None, before=None, enrich=False):
        args = {"dm_channel": dm_channel, "limit": limit, "after": after, "before": before, "enrich": enrich}
        return _command("slack", "dms.history", args)


class _SlackUsers:
    def list(self, limit=None):
        return _command("slack", "users.list", {"limit": limit})

    def info(self, user):
        return _command("slack", "users.info", {"user": user})

    def search(self, query):
        return _command("slack", "users.search", {"query": query})

    def presence(self, user):
        return _command("slack", "users.presence", {"user": user})


class _SlackFiles:
    def list(self, channel=None, user=None, limit=None):
        return _command("slack", "files.list", {"channel": channel, "user": user, "limit": limit})

    def info(self, file):
        return _command("slack", "files.info", {"file": file})


class _SlackToken:
    def test(self):
        return _command("slack", "token.test", {})


class _Slack:
    def __init__(self):
        self.search = _SlackSearch()
        self.me = _SlackMe()
        self.channels = _SlackChannels()
        self.messages = _SlackMessages()
        self.dms = _SlackDms()
        self.users = _SlackUsers()
        self.files = _SlackFiles()
        self.token = _SlackToken()

    def get(self, method, params=None, headers=None):
        return _get("slack", method, params, headers)


class _List:
    def __init__(self, name):
        self._name = name

    def list(self):
        return _command("launchpad", self._name + ".list", {})


class _Sessions:
    def list(self):
        return _command("launchpad", "sessions.list", {})

    def show(self, session_id):
        return _command("launchpad", "sessions.show", {"session_id": session_id})


class _Topics:
    def list(self):
        return _command("launchpad", "topics.list", {})

    def show(self, topic):
        return _command("launchpad", "topics.show", {"topic": topic})


class _Oracle:
    def list(self):
        return _command("launchpad", "oracle.list", {})

    def show(self, slug):
        return _command("launchpad", "oracle.show", {"slug": slug})

    def history(self, slug, failed=False, limit=20):
        return _command("launchpad", "oracle.history", {"slug": slug, "failed": failed, "limit": limit})


class _Launchpad:
    def __init__(self):
        self.folders = _List("folders")
        self.bots = _List("bots")
        self.schedules = _List("schedules")
        self.proposals = _List("proposals")
        self.activity = _List("activity")
        self.profiles = _List("profiles")
        self.presets = _List("presets")
        self.sessions = _Sessions()
        self.topics = _Topics()
        self.oracle = _Oracle()

    def today(self):
        return _command("launchpad", "today", {})

    def notes(self, repo, number):
        return _command("launchpad", "notes", {"repo": repo, "number": number})

    def get(self, path, params=None):
        return _get("launchpad", path, params, None)


class _Tools:
    def __init__(self):
        self.http = _Http()
        self.github = _Api("github")
        self.huggingface = _Api("huggingface")
        self.slack = _Slack()
        self.launchpad = _Launchpad()


tools = _Tools()


def _finish(check, raw):
    """Home's call of check: the arguments as JSON (`now` as Unix seconds), the answer as JSON."""
    data = json.loads(raw)
    kwargs = data["kwargs"]
    if data["now"] is not None:
        kwargs["now"] = datetime.fromtimestamp(data["now"], timezone.utc)
    result = check(**kwargs)
    if result is True or result is False:
        result = verdict(result)
    if not isinstance(result, Verdict):
        name = type(result).__name__
        return json.dumps({"kind": "contract", "message": "check returned " + name + ", not verdict(...), True or False"})
    answer = {
        "kind": "verdict",
        "value": result.value,
        "summary": result.summary,
        "fingerprint": result.fingerprint,
        "links": result.links,
        "items": result.items,
        "cursor": result.cursor,
        "cursor_set": result.cursor_set,
    }
    try:
        return json.dumps(answer)
    except Exception as e:
        return json.dumps({"kind": "contract", "message": "the evidence is not JSON: " + str(e)})
