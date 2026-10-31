"""Space front door, browser setup, one daemon and one serialized recovery worker."""
import asyncio
import base64
import hmac
import json
import os
import secrets
import signal
import time
import uuid
from pathlib import Path

import aiohttp
from aiohttp import web
from huggingface_hub import HfApi

from backup.machine import atomic
from backup.store import Bucket
from backup.worker import Recovery
from gateway.proxy import Proxy
from providers.setup import Logins, ssh_key, public_details
from supervisor.policy import Deployment, verify, MissingBucket
from supervisor.processes import CONTROL, HOME, prepare, environment, spawn
from supervisor.runtime import Runtime
from vault.codec import Vault

ROOT = Path(__file__).resolve().parents[1]
SHELL = ROOT/'gateway'


async def serve():
    deployment = Deployment(os.environ['SPACE_ID'],os.environ.get('LP_SPACE_BUCKET',os.environ['SPACE_ID']),'https://'+os.environ['SPACE_HOST'])
    token = os.environ.get('LP_SPACE_MANAGEMENT_TOKEN','')
    state = {'ready':False,'settings':f'https://huggingface.co/spaces/{deployment.space}/settings',
             'message':'Checking startup.','policy':None,'needs_key':False,'needs_bucket':False,'release':{},'update':None}
    runtime = Runtime(deployment)
    proxy = Proxy(deployment.origin,'')
    logins = Logins()
    recovery = None
    worker = None
    updater = None
    csrf = secrets.token_urlsafe(32)
    actions = asyncio.Lock()
    mutations = asyncio.Lock()
    async def initialize():
        nonlocal recovery, worker, updater
        try:
            if not token: raise ValueError('Add LP_SPACE_MANAGEMENT_TOKEN in Space Secrets, then restart.')
            state['policy'] = await verify(deployment,token)
            prepare()
            release = json.loads((ROOT/'release.json').read_text())
            state['release'] = release
            key = os.environ.get('LP_SPACE_BACKUP_KEY')
            if not key:
                state['needs_key'] = True
                state['message'] = 'Protect your credentials with a dedicated Space Secret. Create the key below; the Space will restart.'
            else:
                control_env = {'HOME':str(CONTROL),'PATH':'/usr/local/bin:/usr/bin:/bin','HF_TOKEN':token,'LANG':'C.UTF-8'}
                bucket = Bucket(deployment.bucket,control_env)
                vault = Vault(key,str(uuid.uuid4()))
                from vault.rotate import recover_rotation
                await recover_rotation(bucket,vault)
                schema = json.loads((ROOT/'schema.json').read_text())['state_schema']
                recovery = Recovery(bucket,vault,schema,release,vault.installation)
                generation = os.environ.get('LP_SPACE_RESTORE_GENERATION') or None
                restore_request = os.environ.get('LP_SPACE_RESTORE_REQUEST',generation)
                if restore_request:
                    from backup.store import safe_name
                    safe_name(restore_request)
                    if '/' in restore_request: raise ValueError('Invalid restore request')
                    if await asyncio.to_thread(bucket.exists,f'backups/restores/{restore_request}.json'): generation = None
                await recovery.recover(generation)
                from supervisor.updates import Updates
                updater = Updates(deployment,token,runtime,recovery,release)
                await updater.load_rollback()
                await runtime.start()
                proxy.token = runtime.token
                await proxy.start(None)
                if not recovery.reconnect:
                    await recovery.checkpoint()
                    if generation: await asyncio.to_thread(bucket.put,f'backups/restores/{restore_request}.json',json.dumps({'generation':generation,'restored_at':time.time()}).encode())
                state['ready'] = True
                state['message'] = 'Your workspace is ready. Connect a tool to get started.'
                worker = asyncio.create_task(recovery.run())
                updater.task = asyncio.create_task(updater.watch())
        except MissingBucket as error:
            state['needs_bucket'] = True
            state['message'] = str(error)
        except ValueError as error:
            state['message'] = str(error)
            await runtime.close()
        except Exception:
            state['message'] = 'Startup could not complete. Check Space Secrets, bucket access and the pinned image; existing backups are preserved.'
            await runtime.close()

    @web.middleware
    async def boundaries(request,handler):
        proxy.guard(request)
        if request.path.startswith('/_space/') and request.method == 'POST':
            supplied = request.headers.get('X-Space-CSRF','')
            if not hmac.compare_digest(supplied,csrf): raise web.HTTPForbidden(text='Reload Space controls before trying again')
        try: response = await handler(request)
        except ValueError as error: response = web.json_response({'error':str(error)},status=409)
        except (aiohttp.ClientError, asyncio.TimeoutError): response = web.json_response({'error':'The service could not complete this action. Try again.'},status=503)
        except web.HTTPException: raise
        except Exception: response = web.json_response({'error':'This action could not complete. Check Hub access and retry from Space controls.'},status=503)
        if not response.prepared:
            response.headers['Cache-Control'] = 'no-store'
            response.headers['X-Content-Type-Options'] = 'nosniff'
            response.headers['Referrer-Policy'] = 'no-referrer'
            if request.path.startswith('/_space/') or not state['ready']:
                response.headers['Content-Security-Policy'] = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-src 'self'; frame-ancestors 'self' https://huggingface.co; object-src 'none'; base-uri 'none'"
        return response

    async def status(request):
        import shutil
        disk=shutil.disk_usage(HOME)
        data = dict(state,csrf=csrf,disk={'free':disk.free,'total':disk.total},backup=recovery.status() if recovery else None,
                    login=logins.status(),maintenance=runtime.maintenance)
        data['update'] = updater.status() if updater else None
        return web.json_response(data)

    async def tools(request):
        if not state['ready']: raise ValueError('Finish startup first')
        checks = await runtime.api('GET','/api/setup/sign-in')
        hf = await runtime.api('GET','/api/setup/trace-backup')
        profiles = (await runtime.api('GET','/api/config'))['config']['profiles']
        return web.json_response({'checks':checks,'hf':hf,'profiles':[{'id':p['id'],'harness':p['harness'],'name':p['name']} for p in profiles]})

    async def action(request):
        nonlocal worker
        body = await request.json()
        if not isinstance(body,dict): raise ValueError('Expected an action object')
        kind = request.match_info['action']
        async with actions:
            if kind == 'create-bucket':
                if not state['needs_bucket']: raise ValueError('No missing bucket was detected')
                await asyncio.to_thread(HfApi(token=token).create_bucket,deployment.bucket,private=True)
                await asyncio.to_thread(HfApi(token=token).restart_space,deployment.space)
                return web.json_response({'message':'Private bucket created. The Space is restarting to verify it.'})
            if kind == 'generate-key':
                if not state['needs_key']: raise ValueError('An encryption key is already configured')
                return web.json_response({'backup_key':base64.b64encode(os.urandom(32)).decode()})
            if kind == 'bootstrap-key':
                if not state['needs_key']: raise ValueError('An encryption key is already configured')
                key = base64.b64encode(os.urandom(32)).decode()
                try:
                    await asyncio.to_thread(HfApi(token=token).add_space_secret,deployment.space,'LP_SPACE_BACKUP_KEY',key)
                except Exception:
                    raise ValueError('Could not save the key. Add LP_SPACE_BACKUP_KEY as a base64-encoded 32-byte random key in Hub Secrets, then restart.') from None
                return web.json_response({'message':'Encryption key saved. Wait for the Space to restart, then refresh.'})
            if not state['ready'] and kind == 'rollback' and updater:
                await updater.rollback(body.get('confirmed') is True)
                return web.json_response({'message':'Rollback requested. Wait for the Space to rebuild.'})
            if not state['ready']: raise ValueError('Finish startup first')
            if runtime.maintenance: raise ValueError('Maintenance is in progress. Wait for the Space to reconnect.')
            if kind == 'login':
                if runtime.maintenance: raise ValueError('Maintenance is in progress')
                await logins.start(body.get('tool'))
            elif kind == 'login-input': await logins.input(body.get('value'))
            elif kind == 'login-cancel': await logins.close()
            elif kind == 'hf-token':
                value = body.get('token')
                if not isinstance(value,str) or not value.startswith('hf_') or len(value)>4096: raise ValueError('Enter a Hugging Face access token')
                try: await asyncio.to_thread(HfApi(token=value).whoami)
                except Exception: raise ValueError('HF could not verify that token') from None
                atomic(HOME/'.cache/huggingface/token',value.encode())
                await recovery.credentials()
            elif kind == 'api-key':
                if body.get('paid') is not True: raise ValueError('Confirm that API usage is billed separately')
                from providers.keys import api_profile
                await api_profile(runtime,body.get('tool'),body.get('provider'),body.get('key'))
                await recovery.credentials()
            elif kind == 'ssh-import':
                from providers.keys import import_ssh
                public = await import_ssh(body.get('private_key'))
                await recovery.credentials()
                return web.json_response(dict(public_details(public),message='SSH key imported and backed up.'))
            elif kind == 'ssh-key':
                public = await ssh_key()
                await recovery.credentials()
                return web.json_response(dict(public_details(public),message='Add this public key to your Git host. Outbound SSH port 22 may be unavailable; prefer Git HTTPS.'))
            elif kind == 'setup-git':
                process = await spawn('gh','auth','setup-git','--hostname','github.com')
                if await process.wait(): raise ValueError('GitHub CLI could not configure HTTPS Git; connect GitHub first')
                await recovery.credentials()
            elif kind == 'profile':
                harness = body.get('tool')
                if harness not in ('codex','claude','opencode'): raise ValueError('Choose an agent tool')
                await runtime.api('POST','/api/setup/profile-from-login',{'harness':harness})
                await recovery.credentials()
            elif kind == 'backup': await recovery.checkpoint()
            elif kind == 'reconnect':
                recovery.reconnect = False
                await recovery.checkpoint()
            elif kind in ('restart','prepare-update','rollback','rotate-key'):
                if logins.status()['running']: raise ValueError('Finish or cancel authorization first')
                runtime.maintenance = True
                try:
                    async with mutations:
                        if not await runtime.idle(): raise ValueError('Agents or terminals are active. Stop them in Launchpad, then try again.')
                    recovery.paused = True
                    async with recovery.lock: pass
                    await runtime.close()
                    checkpoint = await recovery.checkpoint(clean=True)
                    if kind == 'rotate-key':
                        if body.get('confirmed') is not True: raise ValueError('Confirm credential key rotation first')
                        from vault.rotate import rotate
                        await rotate(recovery,HfApi(token=token),deployment.space)
                        await asyncio.to_thread(HfApi(token=token).restart_space,deployment.space)
                    elif kind == 'prepare-update': await updater.apply(checkpoint)
                    elif kind == 'rollback': await updater.rollback(body.get('confirmed') is True)
                    else: await asyncio.to_thread(HfApi(token=token).restart_space,deployment.space)
                except Exception as error:
                    from supervisor.updates import UncertainMaintenance
                    if runtime.process is None and (kind == 'rotate-key' or isinstance(error,UncertainMaintenance)):
                        state['ready'] = False
                        state['message'] = str(error)
                        raise
                    recovery.paused = False
                    if runtime.process is None: await runtime.start()
                    proxy.token = runtime.token
                    runtime.maintenance = False
                    raise
                return web.json_response({'message':'Checkpoint verified. The Space is restarting; keep this tab open.'})
            else: raise ValueError('Unknown Space action')
        return web.json_response({'ok':True})

    async def shell(request): return web.FileResponse(SHELL/'shell.html')
    async def script(request): return web.FileResponse(SHELL/'shell.js')

    async def dispatch(request):
        if not state['ready']: return await shell(request)
        path = proxy.guard(request)
        if path == '/' or (request.method == 'GET' and request.headers.get('Sec-Fetch-Dest') == 'document' and not path.startswith('/api/')):
            from urllib.parse import quote
            raise web.HTTPFound('/_space/shell?route='+quote(request.path_qs if path != '/' else '/agents',safe=''))
        if runtime.maintenance: raise web.HTTPServiceUnavailable(text='Maintenance is in progress; reconnect shortly')
        if request.method == 'POST' and (path == '/api/launch' or path.endswith('/input')):
            age = time.time()-(recovery.checkpoint_at or 0)
            if age > 600: raise web.HTTPServiceUnavailable(text='Recovery is more than ten minutes behind. Open Space controls to retry backup.')
        if path == '/api/config' and request.method in ('PATCH','PUT'):
            body = await request.json()
            backup = body.get('trace_backup',{})
            if (body.get('trace_backup_bucket',deployment.bucket) != deployment.bucket
                or body.get('trace_backup_prefix','traces') != 'traces'
                or backup.get('bucket',deployment.bucket) != deployment.bucket or backup.get('prefix','traces') != 'traces'):
                raise ValueError('Change the Space backup destination in Hub Variables and restart to verify it')
            # Config is bounded JSON, unlike the streaming upload/download routes.
            return web.json_response(await runtime.api(request.method,path,body))
        if path.startswith('/api/profiles/') and path.endswith('/setup'):
            raise ValueError('Connect tools through Space controls so authorization output stays temporary')
        if path == '/sw.js': raise web.HTTPNotFound()
        if request.method not in ('GET','HEAD','OPTIONS'):
            async with mutations:
                if runtime.maintenance: raise web.HTTPServiceUnavailable(text='Maintenance is in progress')
                return await proxy.handle(request)
        return await proxy.handle(request)

    app = web.Application(middlewares=[boundaries],client_max_size=1024*1024)
    app.router.add_get('/_space/status',status)
    app.router.add_get('/_space/tools',tools)
    app.router.add_post('/_space/action/{action}',action)
    app.router.add_get('/_space/shell',shell)
    app.router.add_get('/_space/shell.js',script)
    app.router.add_route('*','/{path:.*}',dispatch)
    runner = web.AppRunner(app,access_log=None)
    await runner.setup()
    await web.TCPSite(runner,'0.0.0.0',7860).start()
    boot = asyncio.create_task(initialize())
    ended = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM,signal.SIGINT): loop.add_signal_handler(sig,ended.set)
    try: await ended.wait()
    finally:
        boot.cancel()
        await asyncio.gather(boot,return_exceptions=True)
        await runner.cleanup()
        for task in (worker, updater.task if updater else None):
            if task: task.cancel()
        await asyncio.gather(*[t for t in (worker,updater.task if updater else None) if t],return_exceptions=True)
        await logins.close()
        await runtime.close()
        if proxy.client: await proxy.close(None)


if __name__ == '__main__':
    asyncio.run(serve())
