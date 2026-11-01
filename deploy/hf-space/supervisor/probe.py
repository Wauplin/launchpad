"""Read-only platform probe; emits no tokens or API bodies. Creates no resources."""
import asyncio
import json
import os

from supervisor.management import SECRET, Management
from supervisor.policy import Deployment, verify


async def main():
    deployment = Deployment(os.environ["SPACE_ID"], os.environ["LP_SPACE_BUCKET"], "https://" + os.environ["SPACE_HOST"])
    try:
        result = await verify(deployment, Management(deployment.space, os.environ[SECRET]).token)
        print(json.dumps({"startup_privacy": "verified", "checked_at": result["checked_at"]}))
    except ValueError as error:
        print(json.dumps({"startup_privacy": "blocked", "reason": str(error)}))
        raise SystemExit(1)
    except Exception:
        print(json.dumps({"startup_privacy": "blocked", "reason": "HF metadata unavailable; inspect permissions and connectivity"}))
        raise SystemExit(1)


if __name__ == "__main__":
    asyncio.run(main())
