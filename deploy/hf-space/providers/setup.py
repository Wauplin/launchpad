"""Fixed ephemeral login processes. Never create Launchpad sessions or log output."""
import asyncio
import errno
import os
import pty
import re
import signal
import termios
import time

from supervisor.processes import spawn, stop, HOME, own

COMMANDS = {
    'codex': ('codex','login','--device-auth'),
    'claude': ('claude','auth','login'),
    'opencode': ('opencode','auth','login'),
    'github': ('gh','auth','login','--hostname','github.com','--git-protocol','https','--web'),
}
ANSI = re.compile(r'\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))')


class Logins:
    def __init__(self):
        self.process = None
        self.tool = None
        self.profile = None
        self.output = ''
        self.master = None
        self.task = None
        self.deadline = None
        self.result = None
        self.lock = asyncio.Lock()

    async def start(self, tool, extra=None, profile=None):
        if tool not in COMMANDS: raise ValueError('Choose a supported tool')
        async with self.lock:
            if self.process and self.process.returncode is None: raise ValueError('Finish or cancel the current connection first')
            if tool == 'codex':
                from pathlib import Path
                from backup.machine import atomic, home_open
                import tomlkit
                config=Path((extra or {}).get('CODEX_HOME',str(HOME/'.codex')))/'config.toml'
                data=tomlkit.parse(home_open(config).read().decode()) if config.exists() else tomlkit.document()
                data['cli_auth_credentials_store']='file'
                atomic(config,tomlkit.dumps(data).encode())
            self.output = ''; self.tool = tool; self.profile = profile; self.result = None
            master, slave = pty.openpty()
            attrs = termios.tcgetattr(slave)
            attrs[3] &= ~termios.ECHO
            termios.tcsetattr(slave, termios.TCSANOW, attrs)
            try: self.process = await spawn(*COMMANDS[tool], stdin=slave, stdout=slave, extra=extra)
            except BaseException:
                os.close(master); raise
            finally: os.close(slave)
            self.master = master
            os.set_blocking(master, False)
            self.deadline = time.time()+900
            self.task = asyncio.create_task(self.read())

    async def read(self):
        try:
            while self.process and self.process.returncode is None:
                if time.time() > self.deadline:
                    await stop(self.process); self.result = 'expired'; break
                try:
                    data = os.read(self.master, 8192)
                    if data: self.output = (self.output+data.decode(errors='replace'))[-24000:]
                except BlockingIOError: pass
                except OSError as error:
                    if error.errno != errno.EIO: raise
                await asyncio.sleep(.1)
            if self.result is None: self.result = 'completed' if self.process.returncode == 0 else 'failed'
        finally:
            if self.master is not None: os.close(self.master); self.master = None

    async def input(self, value):
        if self.master is None: raise ValueError('This authorization has ended')
        if not isinstance(value,str) or len(value)>4096 or '\n' in value or '\r' in value: raise ValueError('Enter a single authorization response')
        os.write(self.master,(value+'\n').encode())

    def status(self):
        text = ANSI.sub('',self.output).replace('\r','')
        # Only the transient setup view receives this bounded buffer. It is never persisted.
        return {'tool':self.tool,'profile':self.profile,'output':text,'expires_at':self.deadline,'result':self.result,
                'running':bool(self.process and self.process.returncode is None)}

    async def close(self):
        await stop(self.process)
        if self.task:
            await self.task
            self.task = None
        self.output = ''; self.tool = None; self.profile = None; self.result = None


async def ssh_key():
    path = HOME/'.ssh/id_ed25519'
    if not path.exists():
        path.parent.mkdir(mode=0o700,exist_ok=True);own(path.parent,True)
        process = await spawn('ssh-keygen','-t','ed25519','-N','','-C','Launchpad Space','-f',str(path))
        if await process.wait(): raise ValueError('SSH key generation failed')
    if not path.with_suffix('.pub').exists(): raise ValueError('Existing key has no public file; import or repair it manually')
    from backup.machine import home_open
    return home_open(path.with_suffix('.pub')).read().decode()


def public_details(public):
    import hashlib
    import base64
    parts=public.split()
    if len(parts)<2 or parts[0] not in ('ssh-ed25519','ssh-rsa','ecdsa-sha2-nistp256','ecdsa-sha2-nistp384','ecdsa-sha2-nistp521'): raise ValueError('Invalid SSH public key')
    blob=base64.b64decode(parts[1],validate=True)
    fingerprint='SHA256:'+base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip('=')
    return {'public_key':public,'fingerprint':fingerprint}


async def profile_login(runtime,tool,profile_id):
    """Use only core-declared account selectors; never inherit control-plane env."""
    if not profile_id: return {}
    from pathlib import Path
    from backup.machine import secure_parent
    config=(await runtime.api('GET','/api/config'))['config']
    profile=next((p for p in config['profiles'] if p['id']==profile_id),None)
    if not profile or profile['harness']!=tool: raise ValueError('Choose an existing profile for this tool')
    descriptors=await runtime.api('GET','/api/harnesses')
    descriptor=next(h for h in descriptors if h['id']==tool)
    if profile['billing']=='pay_as_you_go' and profile.get('env'):
        raise ValueError('This profile uses provider credentials. Reconnect HF below or edit its Credentials in Launchpad Profiles.')
    extra={}
    for key in descriptor['account_env']:
        value=profile.get('env',{}).get(key)
        if not value: continue
        value=value.replace('${HOME}',str(HOME))
        if value.startswith('~/'): value=str(HOME/value[2:])
        path=Path(value)
        if '${' in value or not path.is_absolute() or path==HOME or not path.is_relative_to(HOME): raise ValueError('Use a dedicated account folder under /home/agent for browser sign-in')
        parent,_=secure_parent(path/'.login-placeholder');os.close(parent)
        extra[key]=str(path)
    return extra
