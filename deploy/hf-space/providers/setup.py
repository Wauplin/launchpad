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
        self.output = ''
        self.master = None
        self.task = None
        self.deadline = None
        self.result = None
        self.lock = asyncio.Lock()

    async def start(self, tool):
        if tool not in COMMANDS: raise ValueError('Choose a supported tool')
        async with self.lock:
            if self.process and self.process.returncode is None: raise ValueError('Finish or cancel the current connection first')
            self.output = ''; self.tool = tool; self.result = None
            master, slave = pty.openpty()
            attrs = termios.tcgetattr(slave)
            attrs[3] &= ~termios.ECHO
            termios.tcsetattr(slave, termios.TCSANOW, attrs)
            try: self.process = await spawn(*COMMANDS[tool], stdin=slave, stdout=slave)
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
        return {'tool':self.tool,'output':text,'expires_at':self.deadline,'result':self.result,
                'running':bool(self.process and self.process.returncode is None)}

    async def close(self):
        await stop(self.process)
        if self.task:
            await self.task
            self.task = None
        self.output = ''; self.tool = None; self.result = None


async def ssh_key():
    path = HOME/'.ssh/id_ed25519'
    if not path.exists():
        path.parent.mkdir(mode=0o700,exist_ok=True);own(path.parent,True)
        process = await spawn('ssh-keygen','-t','ed25519','-N','','-C','Launchpad Space','-f',str(path))
        if await process.wait(): raise ValueError('SSH key generation failed')
    if not path.with_suffix('.pub').exists(): raise ValueError('Existing key has no public file; import or repair it manually')
    return path.with_suffix('.pub').read_text()
