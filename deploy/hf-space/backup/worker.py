"""One serialized writer, separate credential acknowledgments and latest-only checkpoints."""
import asyncio
import hashlib
import json
import time
from pathlib import Path

from backup.machine import capture, credential_snapshot, restore, atomic
from backup.store import CREDENTIALS, NEXT, safe_name
from supervisor.processes import HOME
from vault.codec import key_id, seal_id

SEALED = 'credentials/checkpoint.aead'


class Recovery:
    def __init__(self, bucket, vault, schema, release, installation):
        self.bucket, self.vault, self.schema, self.release = bucket, vault, schema, release
        self.installation = installation
        self.lock = asyncio.Lock()
        self.paused = False
        self.checkpoint_at = None
        self.credentials_at = None
        self.last_bytes = None
        self.restore_seconds = None
        self.error = None
        self.credential_hash = None
        self.reconnect = False
        self.reconnect_reason = None
        self.pending_credentials = False
        self.manifest = None  # Last committed or restored manifest; unchanged objects are not uploaded again.
        self.files = None  # Bytes of the last full checkpoint, for the pre-update restore point.
        self.workspace_at = 0.0
        self.payload_digest = None
        self.sealed = None
        self.migrated = False

    async def open_sealed(self, name, envelope):
        """Open a credential object; after a planned key change, its copy under the new key is in `space/next/`."""
        if key_id(envelope) != self.vault.key_id:
            copy = f'{NEXT}/{Path(name).name}'
            if await asyncio.to_thread(self.bucket.exists, copy):
                candidate = await asyncio.to_thread(self.bucket.read, copy)
                if key_id(candidate) == self.vault.key_id and seal_id(candidate) == seal_id(envelope):
                    envelope = candidate
        return self.vault.open_object(envelope)

    async def recover(self, checkpoint=None):
        """Restore the latest state, the pre-update restore point `checkpoint`, or migrate a format 1 backup once."""
        started = time.monotonic()
        if checkpoint:
            manifest = await asyncio.to_thread(self.bucket.pre_update, checkpoint)
        else:
            manifest = await asyncio.to_thread(self.bucket.latest)
        if manifest is None:
            await self.recover_legacy()
        else:
            files = await asyncio.to_thread(self.bucket.restore, manifest)
            self.installation = self.vault.installation = manifest['installation']
            payload, provider_files = {}, None
            try:
                payload = await self.open_sealed(SEALED, files.pop(SEALED))
                if not checkpoint and await asyncio.to_thread(self.bucket.exists, CREDENTIALS):
                    latest = await self.open_sealed(CREDENTIALS, await asyncio.to_thread(self.bucket.read, CREDENTIALS))
                    if latest.get('captured_at', 0) > manifest.get('captured_at', 0): provider_files = latest['files']
            except Exception as error:
                payload = self.lost(error)
            await asyncio.to_thread(restore, files, payload, self.reconnect)
            self.apply_credentials(payload, provider_files)
            self.manifest = manifest
        self.workspace_at = time.monotonic()
        self.restore_seconds = time.monotonic() - started

    async def recover_legacy(self):
        """Format 1: the newest `backups/<generation>/` with its vault. Project files land in the bucket volume."""
        legacy = await asyncio.to_thread(self.bucket.legacy)
        if legacy is None: return
        manifest, files = legacy
        self.installation = self.vault.installation = manifest['installation']
        safe_name(manifest['credentials'])
        if '/' in manifest['credentials']: raise ValueError('Invalid credential generation')
        payload, provider_files = {}, None
        try:
            envelope = await asyncio.to_thread(self.bucket.read, f"credentials/{manifest['credentials']}/vault.aead")
            payload = self.vault.open(manifest['credentials'], envelope)
            latest = await asyncio.to_thread(self.bucket.read_json, 'credentials/latest.json')
            if latest and latest.get('captured_at', 0) > manifest.get('captured_at', 0):
                safe_name(latest['generation'])
                if '/' in latest['generation']: raise ValueError('Invalid credential generation')
                sealed = await asyncio.to_thread(self.bucket.read, f"credentials/{latest['generation']}/vault.aead")
                provider_files = self.vault.open(latest['generation'], sealed)['files']
        except Exception as error:
            payload = self.lost(error)
        print('Migrating the format 1 backup: projects go to the bucket volume once.', flush=True)
        await asyncio.to_thread(restore, files, payload, self.reconnect)
        self.apply_credentials(payload, provider_files)
        self.migrated = True

    def lost(self, error):
        from cryptography.exceptions import InvalidTag
        self.reconnect = True
        if isinstance(error, InvalidTag): self.reconnect_reason = 'Credential authentication failed. The original vault is untouched; ordinary data was recovered.'
        elif isinstance(error, ValueError): self.reconnect_reason = 'Credential key or format differs. Ordinary data was recovered and the old vault is preserved.'
        else: self.reconnect_reason = 'Credential copy is unavailable. Restart to retry, or explicitly reconnect tools; old backups are preserved.'
        return {}

    def apply_credentials(self, payload, provider_files):
        """Newer credential files from the fast path replace the checkpoint's copies."""
        if provider_files is None or self.reconnect: return
        from base64 import b64decode
        from providers.roots import credential
        previous = set(payload.get('files', {})) | set(credential_snapshot())
        for name in previous - set(provider_files):
            if credential(name) or '.git' in Path(name).parts: (HOME/name).unlink(missing_ok=True)
        for name, data in provider_files.items():
            safe_name(name)
            if name != '.config/launchpad/config.toml' and not credential(name) and '.git' not in Path(name).parts: raise ValueError('Unregistered credential root')
            atomic(HOME/name, b64decode(data, validate=True))

    async def checkpoint(self, clean=False, full=False):
        async with self.lock:
            if self.reconnect: raise ValueError('Reconnect credentials before creating a new checkpoint')
            # Agent state is archived every five minutes (and for clean checkpoints); the rest every minute.
            workspace = clean or full or self.manifest is None or time.monotonic() - self.workspace_at >= 300
            files, secrets = await asyncio.to_thread(capture, self.schema, self.release, workspace)
            digest = hashlib.sha256(json.dumps(secrets, sort_keys=True).encode()).hexdigest()
            if digest != self.payload_digest or self.sealed is None or key_id(self.sealed) != self.vault.key_id:
                # Sealing is randomized: reseal only when the payload changed, so an idle machine uploads nothing.
                sealed = self.vault.seal_object(secrets)
                if self.vault.open_object(sealed) != secrets: raise RuntimeError('Credential verification failed')
                self.sealed = sealed
            files[SEALED] = self.sealed
            metadata = dict(self.release, clean=clean)
            manifest = await asyncio.to_thread(self.bucket.commit, files, self.installation, metadata, time.time(), self.manifest)
            if workspace: self.workspace_at = time.monotonic()
            self.files = files if workspace else None
            self.manifest = manifest
            self.payload_digest = digest
            self.last_bytes = manifest['bytes']
            self.checkpoint_at = time.time()
            self.error = None
            return manifest

    async def credentials(self):
        async with self.lock:
            if self.reconnect or self.manifest is None: return
            self.pending_credentials = True
            try:
                files = await asyncio.to_thread(credential_snapshot)
                digest = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
                if digest == self.credential_hash: return
                captured_at = time.time()
                sealed = self.vault.seal_object({'files': files, 'captured_at': captured_at})
                self.vault.open_object(sealed)
                await asyncio.to_thread(self.bucket.put, CREDENTIALS, sealed)
                self.credential_hash = digest
                self.credentials_at = time.time()
            finally:
                self.pending_credentials = False

    async def run(self):
        from watchfiles import awatch
        from providers.roots import credential
        changed = asyncio.Event()
        async def observe():
            # ~/projects is the bucket volume: watching it would read the remote tree.
            async for events in awatch(*[p for p in HOME.iterdir() if p.name != 'projects' and p.is_dir() and not p.is_symlink()],
                                       debounce=1000, step=200, ignore_permission_denied=True):
                if any(credential(str(Path(path).relative_to(HOME))) for _, path in events):
                    changed.set()
        observer = asyncio.create_task(observe())
        last_full = 0
        try:
            while True:
                if self.paused:
                    await asyncio.sleep(1)
                    continue
                try:
                    await self.credentials()
                    if not self.paused and time.monotonic() - last_full >= 60:
                        await self.checkpoint()
                        last_full = time.monotonic()
                except Exception:
                    self.error = 'The last attempt failed. Check bucket access and capacity; retrying automatically.'
                try: await asyncio.wait_for(changed.wait(), 30)
                except asyncio.TimeoutError: pass
                changed.clear()
        finally:
            observer.cancel()
            await asyncio.gather(observer, return_exceptions=True)

    def status(self):
        return {'bytes': self.last_bytes, 'restore_seconds': self.restore_seconds, 'checkpoint_at': self.checkpoint_at,
                'credentials_at': self.credentials_at, 'checkpoint': (self.manifest or {}).get('checkpoint'),
                'error': self.error, 'reconnect': self.reconnect, 'credentials_pending': self.pending_credentials,
                'reconnect_reason': self.reconnect_reason}
