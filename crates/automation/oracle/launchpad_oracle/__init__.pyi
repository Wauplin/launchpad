# launchpad_oracle.pyi: the whole interface an oracle sees (served by Home, used for type checking)
from dataclasses import dataclass
from datetime import datetime
from typing import Any, NoReturn

# A JSON value: dict, list, str, int, float, bool or None (Any, so code can index it without a cast).
JSON = Any

@dataclass(frozen=True)
class Response:
    status_code: int
    text: str | None  # the body decoded as UTF-8, or None when it is not valid UTF-8
    json: JSON  # the text parsed as JSON, or None when there is no text or it is not JSON
    def raise_for_status(self) -> Response: ...  # 4xx or 5xx: the evaluation fails with class http

@dataclass(frozen=True)
class Verdict: ...

def verdict(
    value: bool,
    *,
    summary: str = "",
    fingerprint: str | None = None,
    links: list[str] | None = None,
    items: list[JSON] | None = None,
    cursor: str | None = None,
) -> Verdict: ...
def failed(reason: str) -> NoReturn: ...  # ends the evaluation with Failed (class declared)

class _Http:
    def get(self, url: str, params: dict[str, Any] | None = None, headers: dict[str, str] | None = None) -> Response: ...

class _Api:  # tools.github, tools.huggingface
    def get(self, endpoint: str, params: dict[str, Any] | None = None, headers: dict[str, str] | None = None) -> Response: ...

class _SlackSearch:
    def messages(self, query: str, limit: int | None = None, page: int | None = None) -> dict[str, Any]: ...

class _SlackMe:
    def channels(self, limit: int | None = None, dms: bool = False, unread: bool = False) -> list[dict[str, Any]]: ...

class _SlackChannels:
    def list(self, limit: int | None = None) -> list[dict[str, Any]]: ...
    def info(self, channel: str) -> dict[str, Any]: ...
    def history(
        self,
        channel: str,
        limit: int | None = None,
        after: str | datetime | None = None,
        before: str | datetime | None = None,
        enrich: bool = False,
    ) -> list[dict[str, Any]]: ...
    def members(self, channel: str, limit: int | None = None) -> list[dict[str, Any]]: ...
    def pins(self, channel: str) -> list[dict[str, Any]]: ...

class _SlackMessages:
    def replies(self, channel: str, thread_ts: str, limit: int | None = None) -> list[dict[str, Any]]: ...
    def permalink(self, channel: str, ts: str) -> dict[str, Any]: ...
    def reactions(self, channel: str, ts: str) -> list[dict[str, Any]]: ...

class _SlackDms:
    def list(self, limit: int | None = None) -> list[dict[str, Any]]: ...
    def history(
        self,
        dm_channel: str,
        limit: int | None = None,
        after: str | datetime | None = None,
        before: str | datetime | None = None,
        enrich: bool = False,
    ) -> list[dict[str, Any]]: ...

class _SlackUsers:
    def list(self, limit: int | None = None) -> list[dict[str, Any]]: ...
    def info(self, user: str) -> dict[str, Any]: ...
    def search(self, query: str) -> list[dict[str, Any]]: ...
    def presence(self, user: str) -> dict[str, Any]: ...

class _SlackFiles:
    def list(self, channel: str | None = None, user: str | None = None, limit: int | None = None) -> list[dict[str, Any]]: ...
    def info(self, file: str) -> dict[str, Any]: ...

class _SlackToken:
    def test(self) -> dict[str, Any]: ...

class _Slack:
    search: _SlackSearch
    me: _SlackMe
    channels: _SlackChannels
    messages: _SlackMessages
    dms: _SlackDms
    users: _SlackUsers
    files: _SlackFiles
    token: _SlackToken
    def get(self, method: str, params: dict[str, Any] | None = None, headers: dict[str, str] | None = None) -> Response: ...

class _List:
    def list(self) -> JSON: ...

class _Sessions:
    def list(self) -> JSON: ...
    def show(self, session_id: str) -> JSON: ...

class _Topics:
    def list(self) -> JSON: ...
    def show(self, topic: str) -> JSON: ...

class _Oracle:
    def list(self) -> JSON: ...
    def show(self, slug: str) -> JSON: ...
    def history(self, slug: str, failed: bool = False, limit: int = 20) -> JSON: ...

class _Launchpad:
    folders: _List
    bots: _List
    schedules: _List
    proposals: _List
    activity: _List
    profiles: _List
    presets: _List
    egress: _List
    sessions: _Sessions
    topics: _Topics
    oracle: _Oracle
    def today(self) -> JSON: ...
    def notes(self, repo: str, number: int) -> JSON: ...
    def get(self, path: str, params: dict[str, Any] | None = None) -> Response: ...

class _Tools:
    http: _Http
    github: _Api
    huggingface: _Api
    slack: _Slack
    launchpad: _Launchpad

tools: _Tools
