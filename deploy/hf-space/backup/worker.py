"""One serialized writer, separate credential acknowledgments and full checkpoints."""
import asyncio
import hashlib
import json
import time
import uuid
from pathlib import Path

from backup.machine import capture, credential_snapshot, restore, atomic
from supervisor.processes import HOME


class Recovery:
    def __init__(self, bucket, vault, schema, release, installation):
        self.bucket, self.vault, self.schema, self.release = bucket, vault, schema, release
        self.installation = installation
        self.lock = asyncio.Lock()
        self.paused = False
        self.checkpoint_at = None
        self.credentials_at = None
        self.generation = None
        self.last_bytes = None
        self.restore_seconds = None
        self.error = None
        self.credential_hash = None
        self.credential_generation = None
        self.reconnect = False
        self.reconnect_reason = None
        self.pending_credentials = False
        self.manifest = None  # Last committed or restored manifest; unchanged objects are referenced from it.
        self.workspace_at = 0.0
        self.payload_digest = None

    async def recover(self, generation=None):
        started=time.monotonic()
        if generation is None:
            if not await asyncio.to_thread(self.bucket.exists, 'backups/latest.json'): return
            pointer = json.loads(await asyncio.to_thread(self.bucket.read, 'backups/latest.json'))
            generation = pointer['generation']
        manifest, files = await asyncio.to_thread(self.bucket.restore, generation)
        self.installation = manifest['installation']
        self.vault.installation = self.installation
        from backup.store import safe_name
        safe_name(manifest['credentials'])
        if '/' in manifest['credentials']: raise ValueError('Invalid credential generation')
        payload={}
        provider_files=None
        try:
            envelope=await asyncio.to_thread(self.bucket.read,f"credentials/{manifest['credentials']}/vault.aead")
            payload=self.vault.open(manifest['credentials'],envelope)
            if await asyncio.to_thread(self.bucket.exists,'credentials/latest.json'):
                latest=json.loads(await asyncio.to_thread(self.bucket.read,'credentials/latest.json'))
                if latest.get('captured_at',0) <= manifest.get('captured_at',0): latest = None
                if latest is not None: safe_name(latest['generation'])
                if latest is not None:
                    if '/' in latest['generation']: raise ValueError('Invalid credential generation')
                    sealed=await asyncio.to_thread(self.bucket.read,f"credentials/{latest['generation']}/vault.aead")
                    provider_files=self.vault.open(latest['generation'],sealed)['files']
        except Exception as error:
            from cryptography.exceptions import InvalidTag
            self.reconnect=True
            if isinstance(error,InvalidTag): self.reconnect_reason='Credential authentication failed. The original vault is untouched; ordinary data was recovered.'
            elif isinstance(error,ValueError): self.reconnect_reason='Credential key or format differs. Ordinary data was recovered and the old vault is preserved.'
            else: self.reconnect_reason='Credential generation is unavailable. Restart to retry, or explicitly reconnect tools; old backups are preserved.'
            payload={}
        await asyncio.to_thread(restore,files,payload,self.reconnect)
        if provider_files is not None and not self.reconnect:
            from providers.roots import credential
            previous=set(payload.get('files',{})) | set(credential_snapshot())
            for name in previous-set(provider_files):
                if credential(name) or '.git' in Path(name).parts: (HOME/name).unlink(missing_ok=True)
            from base64 import b64decode
            for name,data in provider_files.items():
                if name!='.config/launchpad/config.toml' and not credential(name) and '.git' not in Path(name).parts: raise ValueError('Unregistered credential root')
                atomic(HOME/name,b64decode(data,validate=True))
        self.generation = generation
        self.manifest = manifest
        self.workspace_at = time.monotonic()
        self.restore_seconds=time.monotonic()-started

    async def checkpoint(self, clean=False, full=False):
        async with self.lock:
            if self.reconnect: raise ValueError('Reconnect credentials before creating a new checkpoint')
            # Workspaces are archived every five minutes (and for clean checkpoints); state every minute.
            workspace = clean or full or self.manifest is None or time.monotonic() - self.workspace_at >= 300
            files, secrets = await asyncio.to_thread(capture, self.schema, self.release, clean, workspace)
            digest = hashlib.sha256(json.dumps(secrets, sort_keys=True).encode()).hexdigest()
            reuse = self.manifest['credentials'] if self.manifest and digest == self.payload_digest else None
            metadata = dict(self.release, clean=clean)
            manifest = await asyncio.to_thread(self.bucket.commit, files, self.vault, secrets, self.installation, metadata,
                                               time.time(), self.manifest, reuse)
            if workspace: self.workspace_at = time.monotonic()
            self.manifest = manifest
            self.payload_digest = digest
            self.generation = manifest['generation']
            self.last_bytes=manifest['bytes']
            self.checkpoint_at = time.time()
            self.error = None
            return manifest

    async def credentials(self):
        async with self.lock:
            if self.reconnect: return
            self.pending_credentials = True
            files = await asyncio.to_thread(credential_snapshot)
            digest = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
            if digest == self.credential_hash:
                self.pending_credentials = False
                return
            generation = uuid.uuid4().hex
            captured_at=time.time()
            sealed = self.vault.seal(generation, {'files': files,'captured_at':captured_at})
            self.vault.open(generation, sealed)
            await asyncio.to_thread(self.bucket.put, f'credentials/{generation}/vault.aead', sealed)
            await asyncio.to_thread(self.bucket.put, 'credentials/latest.json', json.dumps({'generation':generation,'captured_at':captured_at}).encode())
            self.credential_hash = digest
            self.credential_generation = generation
            self.credentials_at = time.time()
            self.pending_credentials = False

    async def run(self):
        from watchfiles import awatch
        from providers.roots import credential
        changed = asyncio.Event()
        async def observe():
            async for events in awatch(HOME,debounce=1000,step=200,ignore_permission_denied=True):
                if any(credential(str(Path(path).relative_to(HOME))) for _,path in events):
                    changed.set()
        observer=asyncio.create_task(observe())
        last_full=0
        try:
            while True:
                if self.paused:
                    await asyncio.sleep(1)
                    continue
                try:
                    await self.credentials()
                    if not self.paused and time.monotonic()-last_full >= 60:
                        await self.checkpoint()
                        last_full=time.monotonic()
                except Exception:
                    self.error='The last attempt failed. Check bucket access and capacity; retrying automatically.'
                try: await asyncio.wait_for(changed.wait(),30)
                except asyncio.TimeoutError: pass
                changed.clear()
        finally:
            observer.cancel()
            await asyncio.gather(observer,return_exceptions=True)

    def status(self):
        return {'bytes':self.last_bytes,'restore_seconds':self.restore_seconds,'checkpoint_at':self.checkpoint_at,'credentials_at':self.credentials_at,
                'generation':self.generation,'error':self.error,'reconnect':self.reconnect,'credentials_pending':self.pending_credentials,'reconnect_reason':self.reconnect_reason}
