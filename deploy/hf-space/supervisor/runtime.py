"""Daemon lifecycle and exact loopback-only management client."""
import asyncio
import secrets

import aiohttp
import tomlkit

from backup.machine import atomic, CONFIG, DATA, home_open
from supervisor.processes import HOME, spawn, stop, stop_agent_children


class Runtime:
    def __init__(self, deployment):
        self.deployment = deployment
        self.process = None
        self.token = None
        self.maintenance = False

    def configure(self):
        config = tomlkit.parse(home_open(CONFIG).read().decode()).unwrap() if CONFIG.exists() else {}
        config['server'] = {'bind':'127.0.0.1','public_origin':self.deployment.origin,'auto_tailnet':False}
        config['trace_backup'] = {'enabled':True,'bucket':self.deployment.bucket,'prefix':'traces'}
        config['projects_root'] = str(HOME/'projects')
        atomic(CONFIG, tomlkit.dumps(config).encode())
        # New each machine; never stored in the bucket.
        self.token = secrets.token_hex(32)
        atomic(DATA/'token', self.token.encode())

    async def start(self):
        if self.process is not None: await self.close()
        self.configure()
        self.process = await spawn('/usr/local/bin/launchpad','daemon',extra={'LAUNCHPAD_TRACE_BACKUP_BUCKET':self.deployment.bucket,'LAUNCHPAD_TRACE_BACKUP_PREFIX':'traces'})
        for _ in range(120):
            if self.process.returncode is not None: raise ValueError('Launchpad exited during startup; inspect the image and restored data')
            try:
                if (await self.api('GET','/api/health')).get('ok'): return
            except (aiohttp.ClientError, asyncio.TimeoutError): pass
            await asyncio.sleep(.5)
        await self.close()
        raise ValueError('Launchpad did not become ready')

    async def api(self, method, path, body=None):
        headers = {'Host':self.deployment.origin.removeprefix('https://'), 'Origin':self.deployment.origin,
                   'Authorization':f'Bearer {self.token}'}
        async with aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=45)) as session:
            async with session.request(method, 'http://127.0.0.1:7317'+path, headers=headers, json=body) as response:
                if response.status >= 400: raise ValueError(f'Launchpad could not complete this action ({response.status})')
                return await response.json()

    async def idle(self):
        sessions = await self.api('GET','/api/sessions?include_assistant=true')
        terminals = await self.api('GET','/api/terminals')
        busy = {'working','starting','running','background','quota_paused','in_terminal'}
        if any(s.get('status') in busy for s in sessions): return False
        return not any(t.get('alive') for t in terminals)

    async def close(self):
        if self.process:
            await stop(self.process)
            await stop_agent_children()
        self.process = None


async def watch_daemon(runtime, state, stage, get_recovery, logins, actions):
    """Keep the control plane usable when the daemon exits; never replay work."""
    def failed():
        return state['ready'] and not runtime.maintenance and (runtime.process is None or runtime.process.returncode is not None)
    while True:
        if failed():
            async with actions:
                if failed():
                    stage('stopped', 'Launchpad stopped unexpectedly. Restart it to continue; unfinished agent turns are never replayed automatically.')
                    recovery = get_recovery()
                    if recovery:
                        recovery.paused = True
                    await logins.close()
                    await runtime.close()
                    if recovery:
                        async with recovery.lock:
                            pass
        await asyncio.sleep(1)
