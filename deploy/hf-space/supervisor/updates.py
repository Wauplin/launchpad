"""Immutable image updates, expected-parent commits and pre-migration rollback."""
import asyncio
import io
import json
import os
import random
import re
import time
import uuid
from pathlib import Path

import aiohttp
from huggingface_hub import HfApi, CommitOperationAdd

from backup.store import RESUMED, UPDATE

# Where releases come from: one place to change, in release.json (`channel`). Both must be publicly readable,
# since duplicated Spaces pull the image and fetch the manifest anonymously.
CHANNEL = json.loads((Path(__file__).resolve().parents[1] / 'release.json').read_text())['channel']
MANIFEST = CHANNEL['manifest']
IMAGE = re.compile(re.escape(CHANNEL['image']) + r'@sha256:[0-9a-f]{64}')


def dockerfile(image):
    if not IMAGE.fullmatch(image): raise ValueError('Release image must use the official immutable image digest')
    return f'# Managed by Launchpad Space controls. Put custom files beside this file.\nFROM {image}\nENV LP_SPACE_IMAGE={image}\n'


class UncertainMaintenance(ValueError):
    pass


# Session states that mean an agent is working (see SessionStatus in crates/core/src/model.rs).
RUNNING = {'starting','working','background'}
RESUMABLE = {'stopped','interrupted'}
# Auto-resume never catches up: a Space that comes back later than this leaves agents stopped.
RESUME_WINDOW = 3600


def summary(session):
    return {'id':session['id'],'title':(session.get('title') or session.get('prompt') or 'Untitled session')[:120]}


async def plan(runtime):
    """What an update would do now: running agents to stop and resume, open terminals, and blockers."""
    sessions = await runtime.api('GET','/api/sessions?include_assistant=true')
    terminals = await runtime.api('GET','/api/terminals')
    blockers = []
    for s in sessions:
        if s.get('status') == 'quota_paused':
            blockers.append(f"{summary(s)['title']} is paused for quota. Stop it in Launchpad first: stopping it here would drop its held messages.")
        elif s.get('status') == 'in_terminal':
            blockers.append(f"{summary(s)['title']} is open in a terminal. Close that terminal first.")
    return {'running':[summary(s) for s in sessions if s.get('status') in RUNNING],
            'terminals':sum(1 for t in terminals if t.get('alive')),'blockers':blockers}


async def stop_agents(runtime, ids, timeout=60, poll=1):
    """Stops each agent like the Stop button, then waits for their turns to end. Returns the ids still running.

    Agents that do not stop in time end when the daemon stops and come back Interrupted, which resumes the same way.
    """
    for id in ids:
        try: await runtime.api('POST',f'/api/sessions/{id}/interrupt')
        except ValueError: pass  # Already finished, or no live process: the status check below decides.
    deadline = time.monotonic()+timeout
    while True:
        sessions = {s['id']:s for s in await runtime.api('GET','/api/sessions?include_assistant=true')}
        left = [id for id in ids if sessions.get(id,{}).get('status') in RUNNING]
        if not left or time.monotonic() >= deadline: return left
        await asyncio.sleep(poll)


async def resume_agents(runtime, stopped):
    """Resumes the stopped agents in their own sessions with "Continue the unfinished turn."; never raises."""
    resumed, failed, finished = [], [], []
    try:
        sessions = {s['id']:s for s in await runtime.api('GET','/api/sessions?include_assistant=true')}
    except Exception:
        return {'resumed':[],'finished':[],'failed':[dict(s,error='Launchpad did not list sessions') for s in stopped]}
    for s in stopped:
        status = sessions.get(s['id'],{}).get('status')
        if status == 'waiting':
            finished.append(s)  # The turn ended on its own before the stop: nothing to continue.
            continue
        if status not in RESUMABLE:
            failed.append(dict(s,error='Session not found' if status is None else f'Session is {status.replace("_"," ")}'))
            continue
        try:
            await runtime.api('POST',f"/api/sessions/{s['id']}/resume",{'continue_turn':True},detail=True)
            resumed.append(s)
        except Exception as error:
            failed.append(dict(s,error=str(error) if isinstance(error,ValueError) else 'Launchpad did not answer'))
    return {'resumed':resumed,'finished':finished,'failed':failed}


class Updates:
    def __init__(self,deployment,token,runtime,recovery,installed):
        self.deployment,self.runtime,self.recovery = deployment,runtime,recovery
        self.api = HfApi(token=token)
        self.installed = dict(installed,image=os.environ.get('LP_SPACE_IMAGE'))
        self.candidate = None
        self.error = None
        self.checked_at = None
        self.etag = None
        self.task = None
        self.run = None  # The owner-confirmed update in progress (supervisor/main.py: update).
        self.rollback_record = None
        self.progress = None  # {'step': stopping|saving|updating|restarting, 'agents': [...], 'to': revision}
        self.result = None  # The last update outcome shown to the owner, including resumed and failed agents.

    def status(self):
        return {'installed':self.installed,'candidate':self.candidate,'error':self.error,
                'checked_at':self.checked_at,'rollback':self.rollback_record,
                'progress':self.progress,'result':self.result}

    async def check(self):
        async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as session:
            async with session.get(MANIFEST,headers={'If-None-Match':self.etag} if self.etag else {}) as response:
                if response.status == 304:
                    self.checked_at=time.time();self.error=None;return
                response.raise_for_status()
                raw = await response.content.read(128*1024+1)
                if len(raw)>128*1024: raise ValueError('Release manifest exceeds the size limit')
                data=json.loads(raw)
                dockerfile(data['image'])
                if not re.fullmatch('[0-9a-f]{40}',data.get('launchpad','')): raise ValueError('Release lacks an exact Launchpad revision')
                if not isinstance(data.get('adapter'),str) or not isinstance(data.get('tools'),dict): raise ValueError('Release metadata is incomplete')
                self.candidate = data if data['launchpad'] != self.installed.get('launchpad') else None
                self.etag=response.headers.get('ETag')
                self.checked_at=time.time();self.error=None

    async def watch(self):
        while True:
            try: await self.check()
            except Exception: self.error='Update check unavailable. Your installed version keeps working.'
            await asyncio.sleep(6*3600+random.uniform(-900,900))

    async def read_file(self,name):
        from huggingface_hub import hf_hub_download
        path = await asyncio.to_thread(hf_hub_download,self.deployment.space,name,repo_type='space',token=self.api.token,force_download=True)
        return Path(path).read_text()

    async def apply(self,checkpoint,expected,agents=()):
        await self.check()
        if not self.candidate: raise ValueError('No newer release is available')
        candidate = dict(self.candidate)
        if expected != candidate['image']: raise ValueError('The available release changed. Review the new candidate before updating.')
        head = await asyncio.to_thread(self.api.space_info,self.deployment.space)
        if not re.fullmatch(r'[0-9a-f]{40}',head.sha or ''): raise ValueError('HF did not return an exact repository revision; nothing was changed')
        previous = await self.read_file('Dockerfile')
        image = self.installed.get('image')
        managed = re.sub(r'ENV LP_SPACE_RESTORE_(?:CHECKPOINT|GENERATION|REQUEST)=[a-f0-9]{32}\n','',previous)
        if not image or managed != dockerfile(image):
            raise ValueError('The managed Dockerfile has custom changes. Compare it with the release template in Hub before updating.')
        # `request` identifies this update; the first boot afterwards claims it once to report and resume `agents`.
        record = {'image':image,'checkpoint':checkpoint['checkpoint'],'revision':head.sha,
                  'candidate':candidate['image'],'at':time.time(),'request':uuid.uuid4().hex,
                  'from':self.installed.get('launchpad'),'to':candidate['launchpad'],'agents':list(agents)}
        # Durable before the repo commit, so recovery is possible even if rebuild interrupts us.
        await asyncio.to_thread(self.recovery.bucket.save_pre_update,checkpoint,self.recovery.files)
        await asyncio.to_thread(self.recovery.bucket.put,UPDATE,json.dumps(record).encode())
        operations=[CommitOperationAdd(path_in_repo='Dockerfile',path_or_fileobj=io.BytesIO(dockerfile(candidate['image']).encode()))]
        try:
            await asyncio.to_thread(self.api.create_commit,self.deployment.space,operations,repo_type='space',parent_commit=head.sha,commit_message='Update Launchpad runtime image')
        except Exception as error:
            response=getattr(error,'response',None)
            if response is not None and 400 <= response.status_code < 500: raise ValueError('HF refused the repository update; refresh the Space revision and try again') from None
            raise UncertainMaintenance('Repository update outcome is uncertain. Inspect Hub commits before another update or rollback.') from None
        self.rollback_record=record

    async def finish(self,runtime,now=None):
        """After a boot that follows an update (or its rollback): report the outcome and resume the stopped agents.

        The claim is written before any resume, so a crash in between never resumes twice.
        """
        record = self.rollback_record or {}
        request = record.get('request')
        if not isinstance(request,str) or not re.fullmatch('[0-9a-f]{32}',request): return
        bucket = self.recovery.bucket
        if ((await asyncio.to_thread(bucket.read_json,RESUMED)) or {}).get('request') == request: return
        await asyncio.to_thread(bucket.put,RESUMED,json.dumps({'request':request,'at':time.time()}).encode())
        agents = record.get('agents') or []
        image = self.installed.get('image')
        outcome = 'updated' if image == record.get('candidate') else 'rolled_back' if image == record.get('image') else 'unknown'
        self.result = {'outcome':outcome,'from':record.get('from'),'to':record.get('to'),'at':time.time(),
                       'resumed':[],'finished':[],'failed':[],'pending':agents}
        if agents and (now or time.time()) - record.get('at',0) > RESUME_WINDOW:
            self.result.update(pending=[],failed=[dict(a,error='The Space came back more than an hour after the update; resume it in Launchpad') for a in agents])
            return
        self.result.update(await resume_agents(runtime,agents),pending=[])

    async def load_rollback(self):
        bucket=self.recovery.bucket
        # backups/update.json is the format 1 record: the image before the projects volume, with its own data.
        record=await asyncio.to_thread(bucket.read_json,UPDATE) or await asyncio.to_thread(bucket.read_json,'backups/update.json')
        if record: self.rollback_record=record

    async def rollback(self,confirmed):
        if not confirmed: raise ValueError('Confirm rollback: writes after the pre-update checkpoint will be lost')
        await self.load_rollback()
        record=self.rollback_record
        if not record: raise ValueError('No pre-update restore point exists')
        variable,value=('CHECKPOINT',record.get('checkpoint','')) if 'checkpoint' in record else ('GENERATION',record.get('generation',''))
        if not re.fullmatch(r'[0-9a-f]{32}',value): raise ValueError('Invalid pre-update restore point')
        head=await asyncio.to_thread(self.api.space_info,self.deployment.space)
        if not re.fullmatch(r'[0-9a-f]{40}',head.sha or ''): raise ValueError('HF did not return an exact repository revision; nothing was changed')
        operations=[CommitOperationAdd(path_in_repo='Dockerfile',path_or_fileobj=io.BytesIO((dockerfile(record['image'])+f"ENV LP_SPACE_RESTORE_{variable}={value}\nENV LP_SPACE_RESTORE_REQUEST={uuid.uuid4().hex}\n").encode()))]
        try:
            await asyncio.to_thread(self.api.create_commit,self.deployment.space,operations,repo_type='space',parent_commit=head.sha,commit_message='Roll back Launchpad image and pre-update data')
        except Exception as error:
            response=getattr(error,'response',None)
            if response is not None and 400 <= response.status_code < 500: raise ValueError('HF refused the repository update; refresh the Space revision and try again') from None
            raise UncertainMaintenance('Repository update outcome is uncertain. Inspect Hub commits before another update or rollback.') from None
