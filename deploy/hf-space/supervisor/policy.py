"""Startup-only deployment policy; owner-approved private/protected compatibility."""
import re
import time
from dataclasses import dataclass

import aiohttp


class MissingBucket(ValueError):
    pass


class Unverified(ValueError):
    """The Space or bucket is not private, or does not match. Only a Hub settings change fixes it."""


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


async def verify(deployment, token, bucket=True):
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
            raise Unverified("This Space is public. Make it private in the Space settings: everyone who can open it gets full access to your agents and credentials.")
        if space.get("id", "").lower() != deployment.space.lower():
            raise Unverified("The Space name reported by Hugging Face does not match this Space.")
        if not bucket:
            return {"checked_at": time.time(), "runtime": space.get("runtime", {})}
        async with session.get(f"https://huggingface.co/api/buckets/{deployment.bucket}", headers=headers) as r:
            if r.status == 404: raise MissingBucket("The backup bucket does not exist yet.")
            r.raise_for_status()
            bucket = await r.json()
        if bucket.get("private") is not True:
            raise Unverified(f"The backup bucket {deployment.bucket} is public. Make it private on the Hub, or set LP_SPACE_BUCKET to another bucket, then restart.")
        if bucket.get("id", "").lower() != deployment.bucket.lower():
            raise Unverified("The bucket name reported by Hugging Face does not match LP_SPACE_BUCKET.")
    return {"checked_at": time.time(), "runtime": space.get("runtime", {})}
