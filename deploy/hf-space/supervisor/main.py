"""Space front door: setup, one Launchpad daemon behind the proxy and one serialized recovery worker.

The browser shell (gateway/shell.*) reads `GET /_space/status` and drives everything through
`POST /_space/action/<name>`. `stage` tells it which screen to show:

  template   the public duplicate template: offer Duplicate, run nothing
  token      no Space sign-in yet, or it expired: sign in with `hf auth login` (saved as a Space Secret)
  blocked    the Space or bucket failed the startup policy; only Hub can fix it
  backups    bucket and/or encryption key missing: one click creates both and restarts
  starting   checks passed, data is being restored and Launchpad is starting
  connect    Launchpad runs but no agent profile exists yet: connect a first agent
  ready      normal use: the app in an iframe under a slim status bar
  stopped    the daemon exited unexpectedly: explicit restart
  error      startup failed for another reason; existing backups are preserved
"""
import asyncio
import base64
import hmac
import json
import os
import secrets
import shutil
import signal
import time
import traceback
import uuid
from pathlib import Path
from urllib.parse import quote

import aiohttp
from aiohttp import web
from huggingface_hub import HfApi
from huggingface_hub.errors import DeviceCodeError

from backup.store import Bucket, safe_name
from backup.worker import Recovery
from gateway.proxy import Proxy
from providers.setup import HARNESSES, SPACE_LOGIN, Logins, profile_login, public_details, ssh_key
from supervisor.management import SECRET, Management
from supervisor.policy import Deployment, MissingBucket, Unverified, verify
from supervisor.processes import CONTROL, HOME, prepare, spawn
from supervisor.runtime import Runtime, watch_daemon
from vault.codec import Vault

ROOT = Path(__file__).resolve().parents[1]
SHELL = ROOT / 'gateway'
STATIC = {
    'shell.js': 'application/javascript',
    'shell.css': 'text/css',
    'theme.js': 'application/javascript',
    'logo.png': 'image/png',
    'vendor/addon-fit.js': 'application/javascript',
    'vendor/xterm.js': 'application/javascript',
    'vendor/xterm.css': 'text/css',
}
CSP = ("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; "
       "frame-src 'self'; frame-ancestors 'self' https://huggingface.co; object-src 'none'; base-uri 'none'")


def human(error):
    """Short, owner-readable text for Hub errors; never echo request IDs or tokens."""
    response = getattr(error, 'response', None)
    code = getattr(response, 'status_code', None) or getattr(error, 'status', None)
    if code in (401, 403):
        return 'Hugging Face refused the request. Check that your account can manage this Space and your buckets.'
    if code == 404:
        return 'Hugging Face could not find this Space or bucket with this sign-in.'
    if code == 429:
        return 'Hugging Face is rate limiting requests. Wait a minute and try again.'
    return 'Hugging Face could not complete the request. Try again in a moment.'


async def serve():
    deployment = Deployment(os.environ['SPACE_ID'], os.environ.get('LP_SPACE_BUCKET', os.environ['SPACE_ID']),
                            'https://' + os.environ['SPACE_HOST'])
    management = Management(deployment.space, os.environ.get(SECRET, ''))
    pending = None  # A finished Space sign-in waiting for Save and continue.
    state = {
        'boot': uuid.uuid4().hex, 'stage': 'starting', 'ready': False, 'message': 'Checking your Space.',
        'space': {'id': deployment.space, 'url': f'https://huggingface.co/spaces/{deployment.space}',
                  'settings': f'https://huggingface.co/spaces/{deployment.space}/settings'},
        'bucket': {'id': deployment.bucket, 'url': f'https://huggingface.co/buckets/{deployment.bucket}'},
        'policy': None, 'needs_key': False, 'needs_bucket': False, 'release': {}, 'update': None, 'signed_in_as': None,
    }
    runtime = Runtime(deployment)
    proxy = Proxy(deployment.origin, '')
    recovery = None
    worker = None
    updater = None
    csrf = secrets.token_urlsafe(32)
    actions = asyncio.Lock()
    # Counts proxied mutations in flight. Maintenance blocks new ones, then waits for these to finish;
    # requests never wait for each other (a slow clone must not block Stop).
    inflight = {'count': 0, 'idle': asyncio.Event()}
    inflight['idle'].set()

    async def drain():
        await inflight['idle'].wait()

    async def forward(request):
        inflight['count'] += 1
        inflight['idle'].clear()
        try:
            return await proxy.handle(request)
        finally:
            inflight['count'] -= 1
            if not inflight['count']:
                inflight['idle'].set()

    def stage(name, message):
        state['stage'], state['message'] = name, message
        state['ready'] = name in ('connect', 'ready')

    async def signed_in(tool, profile, result):
        """After a successful sign-in, create the matching profile when none exists, then seal credentials."""
        nonlocal pending
        if tool == 'space':
            # Keep the new sign-in in memory only until Save and continue writes it to the Space secret.
            try:
                if result == 'completed':
                    pending = Management.from_login(deployment.space, SPACE_LOGIN)
                    user = await asyncio.to_thread(HfApi(token=pending.token).whoami)
                    state['signed_in_as'] = user.get('name')
            except Exception:
                traceback.print_exc()
                pending = None
            finally:
                shutil.rmtree(SPACE_LOGIN, ignore_errors=True)
            return
        if result != 'completed' or not state['ready']:
            return
        try:
            if tool in HARNESSES and not profile:
                profiles = (await runtime.api('GET', '/api/config'))['config']['profiles']
                if not any(p['harness'] == tool for p in profiles):
                    await runtime.api('POST', '/api/setup/profile-from-login', {'harness': tool})
            if tool == 'huggingface':
                from backup.machine import home_open
                from providers.keys import hf_profile
                with home_open(HOME / '.cache/huggingface/token') as file:
                    await hf_profile(runtime, file.read().decode().strip())
            await refresh_profiles()
        except Exception:
            pass  # The connection list still shows the login; the owner can retry from there.
        if recovery:
            await recovery.credentials()

    logins = Logins(on_exit=signed_in)

    async def refresh_profiles():
        profiles = (await runtime.api('GET', '/api/config'))['config']['profiles']
        if state['stage'] == 'connect' and profiles:
            stage('ready', 'Your workspace is ready.')
        elif state['stage'] == 'ready' and not profiles:
            stage('connect', 'Connect an agent to get started.')
        return profiles

    async def initialize():
        nonlocal recovery, worker, updater
        try:
            if os.environ.get('LP_SPACE_TEMPLATE_ID', '').lower() == deployment.space.lower():
                state['duplicate_url'] = f'https://huggingface.co/spaces/{deployment.space}?duplicate=true'
                stage('template', 'Duplicate this Space to get your own private Launchpad.')
                return
            if not management.token:
                stage('token', 'Sign in with Hugging Face so this Space can manage its own backups and secrets.')
                return
            try:
                await management.renew()
            except DeviceCodeError:
                stage('token', 'The Hugging Face sign-in of this Space expired. Sign in again to continue.')
                return
            state['policy'] = await verify(deployment, management.token)
            prepare()
            release = json.loads((ROOT / 'release.json').read_text())
            state['release'] = release
            key = os.environ.get('LP_SPACE_BACKUP_KEY')
            if not key:
                state['needs_key'] = True
                stage('backups', 'Create the encrypted backup for your workspace.')
                return
            stage('starting', 'Restoring your workspace from the backup bucket.')
            control_env = {'HOME': str(CONTROL), 'PATH': '/usr/local/bin:/usr/bin:/bin', 'HF_TOKEN': management.token, 'LANG': 'C.UTF-8'}
            management.listeners.append(lambda value: control_env.update(HF_TOKEN=value))
            bucket = Bucket(deployment.bucket, control_env)
            vault = Vault(key, str(uuid.uuid4()))
            from vault.rotate import recover_rotation
            await recover_rotation(bucket, vault)
            schema = json.loads((ROOT / 'schema.json').read_text())['state_schema']
            recovery = Recovery(bucket, vault, schema, release, vault.installation)
            generation = os.environ.get('LP_SPACE_RESTORE_GENERATION') or None
            restore_request = os.environ.get('LP_SPACE_RESTORE_REQUEST', generation)
            if restore_request:
                safe_name(restore_request)
                if '/' in restore_request:
                    raise ValueError('Invalid restore request')
                if await asyncio.to_thread(bucket.exists, f'backups/restores/{restore_request}.json'):
                    generation = None
            await recovery.recover(generation)
            if generation:
                # Acknowledge the one-shot restore right away and make it the latest checkpoint, so a later
                # restart neither repeats it nor restores newer (migrated) data on this older image.
                await asyncio.to_thread(bucket.put, 'backups/latest.json', json.dumps({'generation': generation}).encode())
                await asyncio.to_thread(bucket.put, f'backups/restores/{restore_request}.json',
                                        json.dumps({'generation': generation, 'restored_at': time.time()}).encode())
            from supervisor.updates import Updates
            updater = Updates(deployment, management.token, runtime, recovery, release)
            management.listeners.append(lambda value: setattr(updater.api, 'token', value))
            asyncio.create_task(management.watch())
            await updater.load_rollback()
            state['message'] = 'Starting Launchpad.'
            await runtime.start()
            proxy.token = runtime.token
            await proxy.start(None)
            if not recovery.reconnect:
                state['message'] = 'Saving the first backup.'
                try:
                    await recovery.checkpoint()
                except Exception:
                    # Never lock the owner out over a backup problem; the worker retries and the bar shows it.
                    traceback.print_exc()
                    recovery.error = 'The first backup failed. Retrying automatically; check the Space logs if this persists.'
            profiles = (await runtime.api('GET', '/api/config'))['config']['profiles']
            if profiles:
                stage('ready', 'Your workspace is ready.')
            else:
                stage('connect', 'Connect an agent to get started.')
            state['started_at'] = time.time()
            worker = asyncio.create_task(recovery.run())
            updater.task = asyncio.create_task(updater.watch())
        except MissingBucket:
            state['needs_bucket'] = True
            state['needs_key'] = not os.environ.get('LP_SPACE_BACKUP_KEY')
            stage('backups', 'Create the encrypted backup for your workspace.')
        except Unverified as error:
            stage('blocked', str(error))
        except aiohttp.ClientResponseError as error:
            stage('error', human(error))
        except ValueError as error:
            print(f'Startup stopped: {error}', flush=True)
            stage('error', str(error))
            await runtime.close()
        except Exception:
            # The Space logs are the owner's only diagnostics; tracebacks carry no secret values.
            traceback.print_exc()
            stage('error', 'Startup could not complete. Check the Space sign-in and bucket access, then restart. Existing backups are preserved.')
            await runtime.close()

    @web.middleware
    async def boundaries(request, handler):
        proxy.guard(request)
        if request.path.startswith('/_space/') and request.method == 'POST':
            supplied = request.headers.get('X-Space-CSRF', '')
            if not hmac.compare_digest(supplied, csrf):
                raise web.HTTPForbidden(text='Reload this page before trying again')
        try:
            response = await handler(request)
        except ValueError as error:
            response = web.json_response({'error': str(error)}, status=409)
        except (aiohttp.ClientError, asyncio.TimeoutError):
            response = web.json_response({'error': 'The service did not answer. Try again.'}, status=503)
        except web.HTTPException:
            raise
        except Exception as error:
            traceback.print_exc()
            response = web.json_response({'error': human(error)}, status=503)
        if not response.prepared:
            response.headers['Cache-Control'] = 'no-store'
            response.headers['X-Content-Type-Options'] = 'nosniff'
            response.headers['Referrer-Policy'] = 'no-referrer'
            if request.path.startswith('/_space/') or not state['ready']:
                response.headers['Content-Security-Policy'] = CSP
        return response

    async def status(request):
        disk = shutil.disk_usage(HOME)
        data = dict(state, csrf=csrf, disk={'free': disk.free, 'total': disk.total},
                    backup=recovery.status() if recovery else None, login=logins.status(),
                    maintenance=runtime.maintenance, update=updater.status() if updater else None)
        return web.json_response(data)

    async def connections(request):
        """One row per thing the owner can connect, computed from Launchpad's own sign-in checks."""
        if not state['ready']:
            raise ValueError('Finish setup first')
        checks = await runtime.api('GET', '/api/setup/sign-in')
        hf = await runtime.api('GET', '/api/setup/trace-backup')
        profiles = await refresh_profiles()

        def signed(tool):
            return any(c['id'] == tool and c.get('signed_in') is True for c in checks)

        def detail(tool):
            check = next((c for c in checks if c['id'] == tool and not c.get('profile')), None)
            return check.get('detail') if check and check.get('signed_in') else None

        hf_profile = any((p.get('models_url') or '').startswith('https://router.huggingface.co') for p in profiles)
        rows = [
            {'id': 'huggingface', 'connected': hf.get('check', {}).get('signed_in') is True,
             'detail': hf.get('account'), 'profile': hf_profile},
            *({'id': tool, 'connected': signed(tool), 'detail': detail(tool),
               'profile': any(p['harness'] == tool for p in profiles)} for tool in HARNESSES),
            {'id': 'github', 'connected': signed('github'), 'detail': detail('github'), 'profile': None},
        ]
        return web.json_response({
            'connections': rows,
            'profiles': [{'id': p['id'], 'harness': p['harness'], 'name': p['name']} for p in profiles],
            'ssh': (HOME / '.ssh/id_ed25519.pub').exists(),
        })

    async def login_socket(request):
        socket = web.WebSocketResponse(max_msg_size=128 * 1024)
        await socket.prepare(request)
        await logins.attach(socket)
        await socket.close()
        return socket

    async def bootstrap():
        """One click: save the Space sign-in, create the bucket and the key, then restart once."""
        signed = pending if state['stage'] == 'token' else management
        if not signed or not signed.token:
            raise ValueError('Sign in with Hugging Face first')
        if signed is pending:
            try:
                await verify(deployment, signed.token, bucket=False)
            except Unverified:
                raise
            except Exception as error:
                raise ValueError(human(error)) from None
        api = HfApi(token=signed.token)
        try:
            if signed is pending or state['needs_bucket']:
                try:
                    await asyncio.to_thread(api.create_bucket, deployment.bucket, private=True)
                except Exception as error:
                    if getattr(getattr(error, 'response', None), 'status_code', None) != 409:
                        raise
                await verify(deployment, signed.token)
            if not os.environ.get('LP_SPACE_BACKUP_KEY'):
                key = base64.b64encode(os.urandom(32)).decode()
                await asyncio.to_thread(api.add_space_secret, deployment.space, 'LP_SPACE_BACKUP_KEY', key,
                                        description='Launchpad credential encryption key. Never share or reuse it.')
            if signed is pending:
                await signed.save()
            await asyncio.to_thread(api.restart_space, deployment.space)
        except (ValueError, web.HTTPException):
            raise
        except Exception as error:
            raise ValueError(human(error)) from None
        stage('restarting', 'Your Space is restarting with its new settings.')
        return {'message': 'Saved. Your Space is restarting.', 'restarting': True}

    async def action(request):
        body = await request.json()
        if not isinstance(body, dict):
            raise ValueError('Expected an action object')
        kind = request.match_info['action']
        async with actions:
            if kind == 'setup':
                if state['stage'] not in ('token', 'backups'):
                    raise ValueError('Setup is already complete')
                return web.json_response(await bootstrap())
            if kind == 'space-login':
                if state['stage'] != 'token':
                    raise ValueError('This Space is already signed in')
                state['signed_in_as'] = None
                await logins.start('space')
                return web.json_response({'ok': True})
            if kind == 'login-cancel' and state['stage'] == 'token':
                await logins.close()
                return web.json_response({'ok': True})
            if kind == 'restart-space' and state['stage'] in ('blocked', 'error', 'stopped', 'restarting') and management.token:
                if body.get('confirmed') is not True:
                    raise ValueError('Confirm the restart first')
                try:
                    await asyncio.to_thread(HfApi(token=management.token).restart_space, deployment.space)
                except Exception as error:
                    raise ValueError(human(error)) from None
                stage('restarting', 'Your Space is restarting.')
                return web.json_response({'message': 'Your Space is restarting.', 'restarting': True})
            if kind == 'generate-key':
                if not state['needs_key']:
                    raise ValueError('An encryption key is already configured')
                return web.json_response({'backup_key': base64.b64encode(os.urandom(32)).decode()})
            if kind == 'restart-daemon' and state['stage'] == 'stopped':
                try:
                    await runtime.start()
                    proxy.token = runtime.token
                except Exception:
                    await runtime.close()
                    raise
                recovery.paused = False
                stage('ready', 'Launchpad restarted from local state.')
                await refresh_profiles()
                return web.json_response({'message': 'Launchpad restarted.'})
            if kind == 'rollback' and not state['ready'] and updater:
                await updater.rollback(body.get('confirmed') is True)
                stage('restarting', 'Rolling back to the previous version.')
                return web.json_response({'message': 'Rollback requested. Your Space is rebuilding.', 'restarting': True})
            if not state['ready']:
                raise ValueError('Finish setup first')
            if runtime.maintenance:
                raise ValueError('Maintenance is in progress. Wait for the Space to reconnect.')
            if kind == 'login':
                if body.get('tool') == 'space':
                    raise ValueError('Choose a supported tool')
                extra = await profile_login(runtime, body.get('tool'), body.get('profile'))
                await logins.start(body.get('tool'), extra=extra, profile=body.get('profile'))
            elif kind == 'login-cancel':
                await logins.close()
            elif kind == 'api-key':
                if body.get('paid') is not True:
                    raise ValueError('Confirm that API usage is billed separately')
                from providers.keys import api_profile
                await api_profile(runtime, body.get('tool'), body.get('provider'), body.get('key'))
                await refresh_profiles()
                await recovery.credentials()
            elif kind == 'ssh-import':
                from providers.keys import import_ssh
                public = await import_ssh(body.get('private_key'))
                await recovery.credentials()
                return web.json_response(dict(public_details(public), message='SSH key imported and backed up.'))
            elif kind == 'ssh-key':
                public = await ssh_key()
                await recovery.credentials()
                return web.json_response(public_details(public))
            elif kind == 'setup-git':
                process = await spawn('gh', 'auth', 'setup-git', '--hostname', 'github.com')
                if await process.wait():
                    raise ValueError('Connect GitHub first, then try again')
                await recovery.credentials()
                return web.json_response({'message': 'Git now uses your GitHub sign-in for HTTPS.'})
            elif kind == 'check-updates':
                await updater.check()
            elif kind == 'backup':
                await recovery.checkpoint(full=True)
                return web.json_response({'message': 'Backup saved.'})
            elif kind == 'reconnect':
                recovery.reconnect = False
                await recovery.checkpoint()
            elif kind in ('restart', 'prepare-update', 'rollback', 'rotate-key'):
                if body.get('confirmed') is not True:
                    raise ValueError('Confirm this action first')
                if logins.status()['running']:
                    raise ValueError('Finish or cancel the current sign-in first')
                runtime.maintenance = True
                try:
                    await drain()
                    if not await runtime.idle():
                        raise ValueError('Agents or terminals are still running. Stop them in Launchpad, then try again.')
                    recovery.paused = True
                    async with recovery.lock:
                        pass
                    await runtime.close()
                    checkpoint = await recovery.checkpoint(clean=True)
                    if kind == 'rotate-key':
                        from vault.rotate import rotate
                        await rotate(recovery, HfApi(token=management.token), deployment.space)
                        await asyncio.to_thread(HfApi(token=management.token).restart_space, deployment.space)
                    elif kind == 'prepare-update':
                        await updater.apply(checkpoint, body.get('image'))
                    elif kind == 'rollback':
                        await updater.rollback(True)
                    else:
                        await asyncio.to_thread(HfApi(token=management.token).restart_space, deployment.space)
                except Exception as error:
                    from supervisor.updates import UncertainMaintenance
                    if runtime.process is None and (kind == 'rotate-key' or isinstance(error, UncertainMaintenance)):
                        stage('error', str(error))
                        raise
                    recovery.paused = False
                    if runtime.process is None:
                        await runtime.start()
                    proxy.token = runtime.token
                    runtime.maintenance = False
                    raise
                stage('restarting', 'Backup verified. Your Space is restarting.')
                return web.json_response({'message': 'Backup verified. Your Space is restarting.', 'restarting': True})
            else:
                raise ValueError('Unknown Space action')
        return web.json_response({'ok': True})

    async def shell(request):
        return web.FileResponse(SHELL / 'shell.html')

    async def asset(request):
        name = request.match_info['name']
        if name not in STATIC:
            raise web.HTTPNotFound()
        return web.FileResponse(SHELL / name, headers={'Content-Type': STATIC[name]})

    async def dispatch(request):
        if not state['ready']:
            if request.path.startswith('/api/') or request.headers.get('Upgrade'):
                raise web.HTTPServiceUnavailable(text='Launchpad is not running yet')
            return await shell(request)
        path = proxy.guard(request)
        if path == '/' or (request.method == 'GET' and request.headers.get('Sec-Fetch-Dest') == 'document'
                           and not path.startswith('/api/')):
            raise web.HTTPFound('/_space/shell?route=' + quote(request.path_qs if path != '/' else '/agents', safe=''))
        if runtime.maintenance:
            raise web.HTTPServiceUnavailable(text='Maintenance is in progress; reconnect shortly')
        if request.method == 'POST' and (path == '/api/launch' or path.endswith('/input')):
            if time.time() - (recovery.checkpoint_at or state['started_at']) > 600:
                raise web.HTTPServiceUnavailable(text='Backups are more than ten minutes behind. Open Space settings to retry the backup.')
        if path == '/api/config' and request.method in ('PATCH', 'PUT'):
            body = await request.json()
            backup = body.get('trace_backup', {})
            if (body.get('trace_backup_bucket', deployment.bucket) != deployment.bucket
                    or body.get('trace_backup_prefix', 'traces') != 'traces'
                    or backup.get('bucket', deployment.bucket) != deployment.bucket
                    or backup.get('prefix', 'traces') != 'traces'):
                raise ValueError('This Space manages the trace destination')
            # Config is bounded JSON, unlike the streaming upload/download routes.
            return web.json_response(await runtime.api(request.method, path, body))
        if path.startswith('/api/profiles/') and path.endswith('/setup'):
            raise ValueError('Sign in through Space settings so sign-in output stays temporary')
        if path == '/sw.js':
            raise web.HTTPNotFound()
        if request.method not in ('GET', 'HEAD', 'OPTIONS'):
            return await forward(request)
        return await proxy.handle(request)

    app = web.Application(middlewares=[boundaries], client_max_size=1024 * 1024)
    app.router.add_get('/_space/status', status)
    app.router.add_get('/_space/connections', connections)
    app.router.add_get('/_space/login', login_socket)
    app.router.add_post('/_space/action/{action}', action)
    app.router.add_get('/_space/shell', shell)
    app.router.add_get('/_space/{name:.+}', asset)
    app.router.add_route('*', '/{path:.*}', dispatch)
    runner = web.AppRunner(app, access_log=None)
    await runner.setup()
    await web.TCPSite(runner, '0.0.0.0', 7860).start()
    boot = asyncio.create_task(initialize())
    watcher = asyncio.create_task(watch_daemon(runtime, state, stage, lambda: recovery, logins, actions))
    ended = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, ended.set)
    try:
        await ended.wait()
    finally:
        boot.cancel()
        watcher.cancel()
        await asyncio.gather(boot, watcher, return_exceptions=True)
        await runner.cleanup()
        tasks = [t for t in (worker, updater.task if updater else None) if t]
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        await logins.close()
        await runtime.close()
        if recovery and state['ready'] and not recovery.reconnect and not runtime.maintenance:
            # HF stops the container on restarts and sleep: save what changed since the last minute, best effort.
            try:
                print('Saving a final backup before stopping.', flush=True)
                await asyncio.wait_for(recovery.checkpoint(full=True), 25)
                print('Final backup saved.', flush=True)
            except Exception:
                traceback.print_exc()
        if proxy.client:
            await proxy.close(None)


if __name__ == '__main__':
    asyncio.run(serve())
