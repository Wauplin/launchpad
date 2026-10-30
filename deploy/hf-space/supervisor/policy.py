"""Startup-only deployment policy. Never infer app privacy from repo privacy."""
import re
import time
from dataclasses import dataclass

import aiohttp


@dataclass(frozen=True)
class Deployment:
    space: str
    bucket: str
    origin: str

    def __post_init__(self):
        for name in (self.space, self.bucket):
            if not re.fullmatch(r"[\w.-]+/[\w.-]+", name, flags=re.ASCII):
                raise ValueError("Use an exact namespace/name for Space and bucket")
        if not re.fullmatch(r"https://[a-z0-9-]+\.hf\.space", self.origin):
            raise ValueError("Use the exact HTTPS hf.space origin without a trailing slash")


async def verify(deployment, token):
    """Fail closed when HF does not expose authoritative three-state visibility.

    `private: true` alone is deliberately insufficient: protected reports it too.
    No owner assertion or anonymous reachability heuristic may bypass this gate.
    """
    headers = {"Authorization": f"Bearer {token}"}
    async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as session:
        async with session.get(f"https://huggingface.co/api/spaces/{deployment.space}", headers=headers) as r:
            r.raise_for_status()
            space = await r.json()
        if space.get("visibility") != "private":
            raise ValueError("HF did not confirm private app visibility. Public, protected and unknown status are blocked.")
        if space.get("id", "").lower() != deployment.space.lower():
            raise ValueError("Space identity did not match")
        async with session.get(f"https://huggingface.co/api/buckets/{deployment.bucket}", headers=headers) as r:
            r.raise_for_status()
            bucket = await r.json()
        if bucket.get("private") is not True:
            raise ValueError("The backup bucket must be private")
        if bucket.get("id", "").lower() != deployment.bucket.lower():
            raise ValueError("Bucket identity did not match")
    return {"checked_at": time.time(), "runtime": space.get("runtime", {})}
