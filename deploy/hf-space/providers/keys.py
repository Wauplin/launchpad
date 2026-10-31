"""Masked credential forms with read-only provider validation and explicit billing."""
import asyncio
import json
import os
from pathlib import Path

import aiohttp

from backup.machine import atomic
from supervisor.processes import HOME, spawn


async def api_profile(runtime,tool,provider,key):
    if tool not in ('codex','claude','opencode') or provider not in ('openai','anthropic'): raise ValueError('Choose a supported tool and provider')
    if (tool=='codex' and provider!='openai') or (tool=='claude' and provider!='anthropic'): raise ValueError('Provider does not match this tool')
    if not isinstance(key,str) or not 8<len(key)<4096: raise ValueError('Enter an API key')
    endpoint = 'https://api.openai.com/v1/models' if provider=='openai' else 'https://api.anthropic.com/v1/models'
    headers = {'Authorization':f'Bearer {key}'} if provider=='openai' else {'x-api-key':key,'anthropic-version':'2023-06-01'}
    try:
        async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as session:
            async with session.get(endpoint,headers=headers) as response:
                if response.status != 200: raise ValueError('Provider could not verify this API key')
    except aiohttp.ClientError: raise ValueError('Provider validation is unavailable; no key was saved') from None
    config=(await runtime.api('GET','/api/config'))['config']
    profiles=config.setdefault('profiles',[])
    profile_id=next((f'{tool}-api-{i}' for i in range(1,1000) if all(p['id']!=f'{tool}-api-{i}' for p in profiles)),None)
    if profile_id is None: raise ValueError('Too many profiles')
    profiles.append({'id':profile_id,'name':f'{tool.title()} · {provider.title()} API','harness':tool,
        'billing':'pay_as_you_go','env':{'OPENAI_API_KEY' if provider=='openai' else 'ANTHROPIC_API_KEY':key},
        'spend_limits':[],'send_credentials':False})
    await runtime.api('PUT','/api/config',config)


async def import_ssh(private):
    if not isinstance(private,str) or len(private)>32768 or 'PRIVATE KEY' not in private: raise ValueError('Paste a supported unencrypted SSH private key')
    destination=HOME/'.ssh/id_ed25519'
    if destination.exists(): raise ValueError('An SSH key already exists; existing keys are never overwritten')
    temporary=HOME/'.ssh/.import-key'
    public=HOME/'.ssh/.import-public'
    atomic(temporary,private.encode())
    # Native validation writes only the public result to a bounded temporary file.
    from supervisor.processes import demote, environment
    try:
        with public.open('wb') as output:
            process=await asyncio.create_subprocess_exec('ssh-keygen','-y','-P','','-f',str(temporary),
                env=environment(),preexec_fn=demote,start_new_session=True,stdin=asyncio.subprocess.DEVNULL,
                stdout=output,stderr=asyncio.subprocess.DEVNULL)
            try: code=await asyncio.wait_for(process.wait(),10)
            except asyncio.TimeoutError:
                from supervisor.processes import stop
                await stop(process);raise ValueError('Key validation timed out') from None
        if code: raise ValueError('Key validation failed. Encrypted keys require an interactive SSH agent and are not imported here.')
        value=public.read_text().strip()
        if not value.startswith(('ssh-ed25519 ','ssh-rsa ','ecdsa-')): raise ValueError('Unsupported key type')
        atomic(destination,private.encode())
        atomic(destination.with_suffix('.pub'),(value+'\n').encode())
        return value
    finally:
        temporary.unlink(missing_ok=True);public.unlink(missing_ok=True)


async def hf_profile(runtime,token):
    """Reuse core's HF provider template instead of copying harness-specific setup."""
    url='https://router.huggingface.co/v1/models'
    try:
        async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=20)) as session:
            async with session.get(url,headers={'Authorization':f'Bearer {token}'}) as response:
                if response.status!=200: raise ValueError('HF token saved, but model access could not be verified. Enable Inference Providers permission and retry.')
                catalog=await response.json()
                if not catalog.get('data'): raise ValueError('HF token saved; no inference models are currently available for this account.')
    except aiohttp.ClientError: raise ValueError('HF token saved, but the model catalog is unavailable. Retry profile creation later.') from None
    config=(await runtime.api('GET','/api/config'))['config']
    profiles=config.setdefault('profiles',[])
    if any(p.get('models_url')==url for p in profiles): return
    descriptors=await runtime.api('GET','/api/harnesses')
    selected=next(((h,t) for h in descriptors for t in h['profile_templates'] if t.get('models_url')==url),None)
    if not selected: raise ValueError('This Launchpad release has no HF provider template. Use native OpenCode authorization with Hugging Face instead.')
    harness,template=selected
    profile_id=next((f'hf-{i}' for i in range(1,1000) if all(p['id']!=f'hf-{i}' for p in profiles)),None)
    if not profile_id: raise ValueError('Too many profiles')
    profiles.append({'id':profile_id,'name':'Hugging Face','harness':harness['id'],
        'env':dict(template['env']),'billing':template['billing'],'models_url':template['models_url'],
        'spend_limits':[],'send_credentials':False})
    await runtime.api('PUT','/api/config',config)
