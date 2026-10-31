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

MANIFEST = 'https://github.com/Wauplin/launchpad/releases/latest/download/hf-space-release.json'
IMAGE = re.compile(r'ghcr\.io/wauplin/launchpad-hf-space@sha256:[0-9a-f]{64}')


def dockerfile(image):
    if not IMAGE.fullmatch(image): raise ValueError('Release image must use the official immutable image digest')
    return f'# Managed by Launchpad Space controls. Put custom files beside this file.\nFROM {image}\nENV LP_SPACE_IMAGE={image}\n'


class UncertainMaintenance(ValueError):
    pass


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
        self.rollback_record = None

    def status(self):
        return {'installed':self.installed,'candidate':self.candidate,'error':self.error,
                'checked_at':self.checked_at,'rollback':self.rollback_record}

    async def check(self):
        async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as session:
            async with session.get(MANIFEST,headers={'If-None-Match':self.etag} if self.etag else {}) as response:
                if response.status == 304:
                    self.checked_at=time.time();return
                response.raise_for_status()
                raw = await response.content.read(128*1024+1)
                if len(raw)>128*1024: raise ValueError('Release manifest exceeds the size limit')
                data=json.loads(raw)
                dockerfile(data['image'])
                if not re.fullmatch('[0-9a-f]{40}',data.get('launchpad','')): raise ValueError('Release lacks an exact Launchpad revision')
                if not isinstance(data.get('adapter'),str) or not isinstance(data.get('tools'),dict): raise ValueError('Release metadata is incomplete')
                self.candidate = data if data['image'] != self.installed.get('image') else None
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

    async def apply(self,checkpoint,expected):
        await self.check()
        if not self.candidate: raise ValueError('No newer release is available')
        if expected != self.candidate['image']: raise ValueError('The available release changed. Review the new candidate before updating.')
        head = await asyncio.to_thread(self.api.space_info,self.deployment.space)
        previous = await self.read_file('Dockerfile')
        image = self.installed.get('image')
        managed = re.sub(r'ENV LP_SPACE_RESTORE_(?:GENERATION|REQUEST)=[a-f0-9]{32}\n','',previous)
        if not image or managed != dockerfile(image):
            raise ValueError('The managed Dockerfile has custom changes. Compare it with the release template in Hub before updating.')
        record = {'image':image,'generation':checkpoint['generation'],'revision':head.sha,
                  'candidate':self.candidate['image'],'at':time.time()}
        # Durable before the repo commit, so recovery is possible even if rebuild interrupts us.
        await asyncio.to_thread(self.recovery.bucket.put,'backups/update.json',json.dumps(record).encode())
        operations=[CommitOperationAdd(path_in_repo='Dockerfile',path_or_fileobj=io.BytesIO(dockerfile(self.candidate['image']).encode()))]
        try:
            await asyncio.to_thread(self.api.create_commit,self.deployment.space,operations,repo_type='space',parent_commit=head.sha,commit_message='Update Launchpad runtime image')
        except Exception as error:
            response=getattr(error,'response',None)
            if response is not None and 400 <= response.status_code < 500: raise ValueError('HF refused the repository update; refresh the Space revision and try again') from None
            raise UncertainMaintenance('Repository update outcome is uncertain. Inspect Hub commits before another update or rollback.') from None
        self.rollback_record=record

    async def load_rollback(self):
        if await asyncio.to_thread(self.recovery.bucket.exists,'backups/update.json'):
            self.rollback_record=json.loads(await asyncio.to_thread(self.recovery.bucket.read,'backups/update.json'))

    async def rollback(self,confirmed):
        if not confirmed: raise ValueError('Confirm rollback: writes after the pre-update checkpoint will be lost')
        await self.load_rollback()
        record=self.rollback_record
        if not record: raise ValueError('No pre-update restore point exists')
        head=await asyncio.to_thread(self.api.space_info,self.deployment.space)
        operations=[CommitOperationAdd(path_in_repo='Dockerfile',path_or_fileobj=io.BytesIO((dockerfile(record['image'])+f"ENV LP_SPACE_RESTORE_GENERATION={record['generation']}\nENV LP_SPACE_RESTORE_REQUEST={uuid.uuid4().hex}\n").encode()))]
        try:
            await asyncio.to_thread(self.api.create_commit,self.deployment.space,operations,repo_type='space',parent_commit=head.sha,commit_message='Roll back Launchpad image and pre-update data')
        except Exception as error:
            response=getattr(error,'response',None)
            if response is not None and 400 <= response.status_code < 500: raise ValueError('HF refused the repository update; refresh the Space revision and try again') from None
            raise UncertainMaintenance('Repository update outcome is uncertain. Inspect Hub commits before another update or rollback.') from None
