"""Fixed sign-in processes in an ephemeral PTY. Never create Launchpad sessions or log output."""
import asyncio
import errno
import fcntl
import json
import os
import pty
import re
import struct
import termios
import time

import aiohttp

from supervisor.processes import spawn, stop, HOME, own

COMMANDS = {
    'codex': ('codex', 'login', '--device-auth'),
    'claude': ('claude', 'auth', 'login'),
    'opencode': ('opencode', 'auth', 'login'),
    'github': ('gh', 'auth', 'login', '--hostname', 'github.com', '--git-protocol', 'https', '--web',
               '--scopes', 'repo,read:org,notifications'),
}
HARNESSES = ('codex', 'claude', 'opencode')
ANSI = re.compile(r'\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-Z\\-_])')
URL = re.compile(r'https://[^\s<>"\x1b\x07]+')
# One-time device codes printed by gh ("ABCD-1234") and Codex ("ABCD-12345").
CODE = re.compile(r'\b[A-Z0-9]{4}-[A-Z0-9]{4,5}\b')
LIMIT = 64 * 1024


class Logins:
    """One sign-in process at a time in a real PTY, streamed to browser terminals.

    Output stays in this bounded memory buffer and the connected sockets; it is never logged,
    persisted or turned into a Launchpad session.
    """

    def __init__(self, on_exit=None):
        self.process = None
        self.tool = None
        self.profile = None
        self.output = b''
        self.master = None
        self.task = None
        self.deadline = None
        self.result = None
        self.sockets = set()
        self.on_exit = on_exit
        self.lock = asyncio.Lock()

    async def start(self, tool, extra=None, profile=None):
        if tool not in COMMANDS:
            raise ValueError('Choose a supported tool')
        async with self.lock:
            if self.process and self.process.returncode is None:
                raise ValueError('Finish or cancel the current sign-in first')
            if self.task:
                await self.task
            if tool == 'codex':
                from pathlib import Path
                import tomlkit
                from backup.machine import atomic, home_open
                config = Path((extra or {}).get('CODEX_HOME', str(HOME / '.codex'))) / 'config.toml'
                data = tomlkit.parse(home_open(config).read().decode()) if config.exists() else tomlkit.document()
                data['cli_auth_credentials_store'] = 'file'
                atomic(config, tomlkit.dumps(data).encode())
            self.output = b''
            self.tool, self.profile, self.result = tool, profile, None
            master, slave = pty.openpty()
            self.resize(master, 24, 100)
            try:
                self.process = await spawn(*COMMANDS[tool], stdin=slave, stdout=slave, extra=extra)
            except BaseException:
                os.close(master)
                raise
            finally:
                os.close(slave)
            self.master = master
            os.set_blocking(master, False)
            self.deadline = time.time() + 900
            self.task = asyncio.create_task(self.read())

    @staticmethod
    def resize(fd, rows, cols):
        rows, cols = max(5, min(int(rows), 200)), max(20, min(int(cols), 400))
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

    async def broadcast(self, data):
        for socket in list(self.sockets):
            try:
                await socket.send_bytes(data)
            except Exception:
                self.sockets.discard(socket)

    async def read(self):
        loop = asyncio.get_running_loop()
        readable = asyncio.Event()
        loop.add_reader(self.master, readable.set)
        try:
            while True:
                if time.time() > self.deadline:
                    await stop(self.process)
                    self.result = 'expired'
                    break
                try:
                    await asyncio.wait_for(readable.wait(), 1)
                except asyncio.TimeoutError:
                    if self.process.returncode is not None:
                        break
                    continue
                readable.clear()
                try:
                    data = os.read(self.master, 65536)
                except BlockingIOError:
                    continue
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
                if not data:
                    break
                self.output = (self.output + data)[-LIMIT:]
                await self.broadcast(data)
            await self.process.wait()
            if self.result is None:
                self.result = 'completed' if self.process.returncode == 0 else 'failed'
        finally:
            loop.remove_reader(self.master)
            os.close(self.master)
            self.master = None
            for socket in list(self.sockets):
                await socket.close()
            if self.on_exit:
                await self.on_exit(self.tool, self.profile, self.result)

    async def attach(self, socket):
        """Replay the bounded buffer, then stream; text frames are keystrokes or a resize."""
        if self.output:
            await socket.send_bytes(self.output)
        if self.master is None:
            return
        self.sockets.add(socket)
        try:
            async for message in socket:
                if message.type != aiohttp.WSMsgType.TEXT or self.master is None:
                    continue
                data = json.loads(message.data)
                if 'input' in data and isinstance(data['input'], str) and len(data['input']) <= 65536:
                    os.write(self.master, data['input'].encode())
                elif 'rows' in data and 'cols' in data:
                    self.resize(self.master, data['rows'], data['cols'])
        finally:
            self.sockets.discard(socket)

    def status(self):
        text = ANSI.sub('', self.output.decode(errors='replace')).replace('\r', '')
        links = list(dict.fromkeys(URL.findall(text)))
        codes = CODE.findall(text)
        # Only the setup view receives these values. They are never persisted.
        return {'tool': self.tool, 'profile': self.profile, 'expires_at': self.deadline, 'result': self.result,
                'running': bool(self.process and self.process.returncode is None),
                'link': links[-1] if links else None, 'code': codes[-1] if codes else None}

    async def close(self):
        await stop(self.process)
        if self.task:
            try:
                await self.task
            except Exception:
                pass  # A failed reader must not take down its caller (the daemon watcher, shutdown).
            self.task = None
        self.output = b''
        self.tool = self.profile = self.result = None


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
