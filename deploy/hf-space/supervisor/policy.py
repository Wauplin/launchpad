"""Startup-only deployment policy; owner-approved private/protected compatibility."""
import re
import time
from dataclasses import dataclass

import aiohttp


class MissingBucket(ValueError):
    pass


@dataclass(frozen=True)
class Deployment:
    space: str
    bucket: str
    origin: str

    def __post_init__(self):
        for name in (self.space, self.bucket):
            if not re.fullmatch(r"[\w.-]+/[\w.-]+", name, flags=re.ASCII) or any(p in (".", "..") for p in name.split("/")):
                raise ValueError("Use an exact namespace/name for Space and bucket")
        if not re.fullmatch(r"https://[a-z0-9-]+\.hf\.space", self.origin):
            raise ValueError("Use the exact HTTPS hf.space origin without a trailing slash")


async def verify(deployment, token):
    """Owner-approved temporary policy: HF private=true includes protected Spaces.

    Keep this decision here so a future Hub visibility fix needs one change.
    Public and unknown responses, and public/unknown buckets, remain rejected.
    """
    headers = {"Authorization": f"Bearer {token}"}
    async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as session:
        async with session.get(f"https://huggingface.co/api/spaces/{deployment.space}", headers=headers) as r:
            r.raise_for_status()
            space = await r.json()
        if space.get("private") is not True:
            raise ValueError("HF did not confirm private/protected repository visibility. Public and unknown status are blocked.")
        if space.get("id", "").lower() != deployment.space.lower():
            raise ValueError("Space identity did not match")
        async with session.get(f"https://huggingface.co/api/buckets/{deployment.bucket}", headers=headers) as r:
            if r.status == 404: raise MissingBucket("Create a private backup bucket to continue. Existing buckets are never changed.")
            r.raise_for_status()
            bucket = await r.json()
        if bucket.get("private") is not True:
            raise ValueError("The backup bucket must be private")
        if bucket.get("id", "").lower() != deployment.bucket.lower():
            raise ValueError("Bucket identity did not match")
    return {"checked_at": time.time(), "runtime": space.get("runtime", {})}
