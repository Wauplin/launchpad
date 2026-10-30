"""A root control plane, an unprivileged stable agent home, and an allowlisted env."""
import asyncio
import os
import pwd
import signal
from pathlib import Path

HOME = Path('/home/agent')
CONTROL = Path('/run/launchpad-space')


def environment():
    # Never inherit platform secrets, the management token, or the vault key.
    return {'HOME': str(HOME), 'USER': 'agent', 'LOGNAME': 'agent', 'SHELL': '/bin/bash',
            'PATH': '/usr/local/bin:/usr/bin:/bin', 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color',
            'CODEX_HOME': str(HOME / '.codex'), 'HF_HOME': str(HOME / '.cache/huggingface'),
            'BROWSER': '/bin/true', 'GH_BROWSER': '/bin/true', 'LAUNCHPAD_DELEGATED_ACCESS': 'true'}


def demote():
    user = pwd.getpwnam('agent')
    os.setgroups([])
    os.setgid(user.pw_gid)
    os.setuid(user.pw_uid)


def own(path, directory=False):
    user = pwd.getpwnam('agent')
    os.chown(path, user.pw_uid, user.pw_gid)
    os.chmod(path, 0o700 if directory else 0o600)


def prepare():
    if os.getuid() != 0:
        raise ValueError('The control plane must run as root; agents run as the separate agent user')
    CONTROL.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(CONTROL, 0o700)
    HOME.mkdir(mode=0o700, exist_ok=True)
    own(HOME, True)
    for name in ('.config/launchpad', '.local/share/launchpad', 'projects', '.codex'):
        path = HOME / name
        path.mkdir(parents=True, exist_ok=True)
        for parent in [path, *path.parents]:
            if parent == HOME: break
            own(parent, True)
    config = HOME / '.codex/config.toml'
    if not config.exists():
        config.write_text('cli_auth_credentials_store = "file"\n')
        own(config)


async def spawn(*args, stdin=None, stdout=None):
    return await asyncio.create_subprocess_exec(*args, cwd=HOME, env=environment(),
        preexec_fn=demote, start_new_session=True, stdin=stdin or asyncio.subprocess.DEVNULL,
        stdout=stdout or asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.STDOUT)


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
