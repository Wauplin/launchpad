"""A root control plane, an unprivileged stable agent home, and an allowlisted env."""
import asyncio
import os
import pwd
import signal
from pathlib import Path

HOME = Path('/home/agent')
# A live bucket volume (the bucket's `projects/` folder) once setup has added it to the Space.
PROJECTS = HOME / 'projects'
CONTROL = Path('/run/launchpad-space')


def environment():
    # Never inherit platform secrets, the management token, or the vault key.
    return {'HOME': str(HOME), 'USER': 'agent', 'LOGNAME': 'agent', 'SHELL': '/bin/bash',
            'PATH': '/usr/local/bin:/usr/bin:/bin', 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color',
            'CODEX_HOME': str(HOME / '.codex'), 'HF_HOME': str(HOME / '.cache/huggingface'),
            'BROWSER': '/bin/true', 'GH_BROWSER': '/bin/true', 'LAUNCHPAD_DELEGATED_ACCESS': 'true', 'LAUNCHPAD_EXTERNAL_SETUP':'/_space/shell?setup=1'}


def demote():
    user = pwd.getpwnam('agent')
    os.setgroups([])
    os.setgid(user.pw_gid)
    os.setuid(user.pw_uid)


def own(path, directory=False):
    user=pwd.getpwnam('agent')
    flags=os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
    if directory: flags |= os.O_DIRECTORY
    fd=os.open(path,flags)
    try:
        os.fchown(fd,user.pw_uid,user.pw_gid)
        os.fchmod(fd,0o700 if directory else 0o600)
    finally: os.close(fd)


def prepare():
    if os.getuid() != 0:
        raise ValueError('The control plane must run as root; agents run as the separate agent user')
    CONTROL.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(CONTROL, 0o700)
    HOME.mkdir(mode=0o700, exist_ok=True)
    own(HOME, True)
    for name in ('.config/launchpad', '.local/share/launchpad', 'projects', '.codex'):
        path = HOME / name
        # The volume's owner and modes belong to the mount; startup checks that the agent can write to it.
        if os.path.ismount(path): continue
        path.mkdir(parents=True, exist_ok=True)
        for parent in [path, *path.parents]:
            if parent == HOME: break
            own(parent, True)
    config = HOME / '.codex/config.toml'
    if not config.exists():
        config.write_text('cli_auth_credentials_store = "file"\n')
        own(config)


async def spawn(*args, stdin=None, stdout=None, extra=None):
    child_env=environment()
    child_env.update(extra or {})
    return await asyncio.create_subprocess_exec(*args, cwd=HOME, env=child_env,
        preexec_fn=demote, start_new_session=True, stdin=stdin or asyncio.subprocess.DEVNULL,
        stdout=stdout or asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.STDOUT)


async def spawn_control(*args, home, stdin, stdout, extra=None):
    """A root process with its own private home: only the Space's own Hub sign-in, never agent work."""
    child_env = {'HOME': str(home), 'HF_HOME': str(home), 'PATH': '/usr/local/bin:/usr/bin:/bin',
                 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color', **(extra or {})}
    return await asyncio.create_subprocess_exec(*args, cwd=home, env=child_env, start_new_session=True,
        stdin=stdin, stdout=stdout, stderr=asyncio.subprocess.STDOUT)


async def stop(process):
    if not process or process.returncode is not None: return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        await asyncio.wait_for(process.wait(), 15)
    except asyncio.TimeoutError:
        os.killpg(process.pid, signal.SIGKILL)
        await process.wait()
    except ProcessLookupError:
        await process.wait()


async def stop_agent_children():
    """Container-only cleanup: the agent UID owns no control-plane processes."""
    uid=pwd.getpwnam('agent').pw_uid
    def owned(pid):
        try: return Path(f'/proc/{pid}').stat().st_uid == uid
        except FileNotFoundError: return False
    pids=[int(p.name) for p in Path('/proc').iterdir() if p.name.isdigit() and owned(p.name)]
    for pid in pids:
        try: os.kill(pid,signal.SIGTERM)
        except ProcessLookupError: pass
    for _ in range(20):
        if not any(owned(pid) for pid in pids): return
        await asyncio.sleep(.1)
    for pid in pids:
        if owned(pid):
            try: os.kill(pid,signal.SIGKILL)
            except ProcessLookupError: pass
