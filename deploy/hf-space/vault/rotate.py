"""Owner-clicked planned rotation. The current ciphertext stays; copies under the new key wait in `space/next/`."""
import asyncio
import base64
import json
import os

from backup.store import CREDENTIALS, NEXT
from vault.codec import Vault, key_id, seal_id

RECORD = f'{NEXT}/rotation.json'


async def rotate(recovery, api, space):
    async with recovery.lock:
        bucket = recovery.bucket
        record = await asyncio.to_thread(bucket.read_json, RECORD)
        if record and record.get('state') == 'pending' and record.get('key_id') == recovery.vault.key_id:
            raise ValueError('A previous key change from this key has an uncertain outcome. Inspect Hub Secrets before trying again.')
        manifest = recovery.manifest
        if not manifest or recovery.sealed is None:
            raise ValueError('Save a backup before changing the key')
        key = base64.b64encode(os.urandom(32)).decode()
        new = Vault(key, recovery.installation)
        # Same seal IDs under the new key: startup matches each copy to the object it replaces.
        copies = {'checkpoint.aead': recovery.sealed}
        if await asyncio.to_thread(bucket.exists, CREDENTIALS):
            copies['credentials.aead'] = await asyncio.to_thread(bucket.read, CREDENTIALS)
        for name, envelope in copies.items():
            if key_id(envelope) != recovery.vault.key_id: continue
            payload = recovery.vault.open_object(envelope)
            sealed = new.seal_object(payload, seal_id(envelope))
            if new.open_object(sealed) != payload: raise RuntimeError('Credential verification failed')
            await asyncio.to_thread(bucket.put, f'{NEXT}/{name}', sealed)
        claim = {'state': 'pending', 'key_id': recovery.vault.key_id, 'new_key_id': new.key_id}
        await asyncio.to_thread(bucket.put, RECORD, json.dumps(claim).encode())
        try:
            # This mutation may restart the container. Both keys can read the credentials from here on.
            await asyncio.to_thread(api.add_space_secret, space, 'LP_SPACE_BACKUP_KEY', key)
        except Exception as error:
            response = getattr(error, 'response', None)
            if response is not None and 400 <= response.status_code < 500:
                await asyncio.to_thread(bucket.put, RECORD, json.dumps(dict(claim, state='refused')).encode())
                raise ValueError('Hugging Face refused the new key; nothing changed') from None
            # Never auto-retry an uncertain secret change; the pending claim blocks a second rotation.
            raise ValueError('Key change did not finish cleanly. Inspect Hub Secrets before restarting; the current backup is unchanged.') from None


async def finish_rotation(bucket, vault):
    """At startup under the new key, acknowledge the rotation so a later one is allowed."""
    record = await asyncio.to_thread(bucket.read_json, RECORD)
    if record and record.get('state') == 'pending' and record.get('new_key_id') == vault.key_id:
        await asyncio.to_thread(bucket.put, RECORD, json.dumps(dict(record, state='done')).encode())
