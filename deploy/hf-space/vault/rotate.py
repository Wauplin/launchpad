"""Owner-clicked planned rotation. Keep all old ciphertext and verify new copies."""
import asyncio
import base64
import json
import os
import uuid

from huggingface_hub import HfApi
from vault.codec import Vault


async def rotate(recovery,api,space):
    async with recovery.lock:
        bucket=recovery.bucket
        old_latest=await asyncio.to_thread(bucket.read,'backups/latest.json')
        old_credentials=await asyncio.to_thread(bucket.read,'credentials/latest.json') if await asyncio.to_thread(bucket.exists,'credentials/latest.json') else None
        rotation=uuid.uuid4().hex
        if await asyncio.to_thread(bucket.exists,'credentials/rotation-pending.json'):
            pending=json.loads(await asyncio.to_thread(bucket.read,'credentials/rotation-pending.json'))
            complete=json.loads(await asyncio.to_thread(bucket.read,'credentials/rotation-complete.json')) if await asyncio.to_thread(bucket.exists,'credentials/rotation-complete.json') else {}
            if pending.get('rotation')!=complete.get('rotation') or complete.get('new_key_id')!=recovery.vault.key_id:
                raise ValueError('A previous key change needs inspection in Hub; do not repeat an uncertain rotation')
        await asyncio.to_thread(bucket.put,'credentials/rotation-pending.json',json.dumps({'rotation':rotation,'key_id':recovery.vault.key_id}).encode())
        key=base64.b64encode(os.urandom(32)).decode()
        new=Vault(key,recovery.installation)
        # Snapshot the object list before writing any new generation.
        objects=await asyncio.to_thread(lambda:list(HfApi(token=bucket.environment['HF_TOKEN']).list_bucket_tree(bucket.bucket_id,recursive=True)))
        manifests=[obj.path for obj in objects if obj.path.startswith('backups/') and obj.path.endswith('/manifest.json')]
        vaults=[obj.path for obj in objects if obj.path.startswith('credentials/') and obj.path.endswith('/vault.aead')]
        try:
            # Re-encrypt retained credential-only generations as well as checkpoint vaults.
            mapping={}
            for path in vaults:
                original=path.split('/')[1]
                envelope=await asyncio.to_thread(bucket.read,path)
                if json.loads(envelope).get('key_id') != recovery.vault.key_id: continue
                payload=recovery.vault.open(original,envelope)
                generation=uuid.uuid4().hex
                sealed=new.seal(generation,payload);new.open(generation,sealed)
                await asyncio.to_thread(bucket.put,f'credentials/{generation}/vault.aead',sealed)
                mapping[original]=generation
            current=json.loads(old_latest)['generation']
            manifests.sort(key=lambda name:name==f'backups/{current}/manifest.json')
            for path in manifests:
                generation=path.split('/')[1]
                manifest,files=await asyncio.to_thread(bucket.restore,generation)
                envelope=await asyncio.to_thread(bucket.read,f"credentials/{manifest['credentials']}/vault.aead")
                if json.loads(envelope).get('key_id') != recovery.vault.key_id: continue
                payload=recovery.vault.open(manifest['credentials'],envelope)
                await asyncio.to_thread(bucket.commit,files,new,payload,recovery.installation,manifest['release'])
            if old_credentials:
                source=json.loads(old_credentials)['generation']
                await asyncio.to_thread(bucket.put,'credentials/latest.json',json.dumps({'generation':mapping[source]}).encode())
            await asyncio.to_thread(bucket.put,'credentials/rotation-pending.json',json.dumps({'rotation':rotation,'key_id':recovery.vault.key_id,'new_key_id':new.key_id,'old_backup':json.loads(old_latest),'old_credentials':json.loads(old_credentials) if old_credentials else None,'new_backup':json.loads(await asyncio.to_thread(bucket.read,'backups/latest.json')),'new_credentials':json.loads(await asyncio.to_thread(bucket.read,'credentials/latest.json')) if old_credentials else None}).encode())
            # This mutation may restart the container. Both complete key generations already exist.
            await asyncio.to_thread(api.add_space_secret,space,'LP_SPACE_BACKUP_KEY',key)
        except Exception:
            # A request to HF Secrets can have an uncertain outcome. Preserve the pending claim;
            # never auto-retry or overwrite any immutable vault generation.
            await asyncio.to_thread(bucket.put,'backups/latest.json',old_latest)
            if old_credentials: await asyncio.to_thread(bucket.put,'credentials/latest.json',old_credentials)
            raise ValueError('Key change did not finish cleanly. Inspect Hub Secrets and the rotation record before restarting; all prior backups were retained.') from None
        # The new runtime can acknowledge this claim using its key identity.
        await asyncio.to_thread(bucket.put,'credentials/rotation-complete.json',json.dumps({'rotation':rotation,'new_key_id':new.key_id}).encode())


async def recover_rotation(bucket,vault):
    if not await asyncio.to_thread(bucket.exists,'credentials/rotation-pending.json'): return
    record=json.loads(await asyncio.to_thread(bucket.read,'credentials/rotation-pending.json'))
    if record.get('new_key_id') == vault.key_id:
        backup,credentials=record.get('new_backup'),record.get('new_credentials')
    elif record.get('key_id') == vault.key_id:
        backup,credentials=record.get('old_backup'),record.get('old_credentials')
    else: return # Genuine key loss follows ordinary reconnect recovery.
    complete=json.loads(await asyncio.to_thread(bucket.read,'credentials/rotation-complete.json')) if await asyncio.to_thread(bucket.exists,'credentials/rotation-complete.json') else {}
    if complete.get('rotation')==record.get('rotation'): return
    if backup: await asyncio.to_thread(bucket.put,'backups/latest.json',json.dumps(backup).encode())
    if credentials: await asyncio.to_thread(bucket.put,'credentials/latest.json',json.dumps(credentials).encode())
    if record.get('new_key_id') == vault.key_id:
        await asyncio.to_thread(bucket.put,'credentials/rotation-complete.json',json.dumps({'rotation':record['rotation'],'new_key_id':vault.key_id}).encode())
