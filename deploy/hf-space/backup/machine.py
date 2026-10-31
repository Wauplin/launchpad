"""Recoverable local files plus separately sealed credentials, at stable paths."""
import base64
import pwd
import io
import json
import os
import sqlite3
import tarfile
import tempfile
import time
import uuid
from pathlib import Path

import tomlkit

from backup.projection import config_split, config_join, project_db, restore_db, REF
from backup.snapshot import materialize, sqlite_snapshot
from backup.store import safe_name
from providers.roots import credential, NATIVE, ROOTS, classify_git
from supervisor.processes import HOME, own

DATA = HOME / '.local/share/launchpad'
CONFIG = HOME / '.config/launchpad/config.toml'
EXCLUDED = {'token', 'vapid.json', 'state.db', 'state.db-wal', 'state.db-shm', 'cache.db', 'cache.db-wal', 'cache.db-shm'}
NATIVE_RUNTIME = {HOME/name for name in NATIVE}
SKIP_DIRS = {'node_modules', 'target', '__pycache__', '.venv', '.cache', '.npm'}


def atomic(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_symlink() or not path.resolve().is_relative_to(HOME):
        raise ValueError('Restore path leaves the agent home')
    for parent in [path.parent, *path.parent.parents]:
        if parent == HOME: break
        own(parent, True)
    temporary = path.with_name(path.name + '.lp-restore')
    with temporary.open('wb') as f:
        os.chmod(temporary, 0o600)
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    own(temporary)
    os.replace(temporary, path)


def files_under(roots):
    seen = set()
    for root in roots:
        if root.is_file() or root.is_symlink(): candidates = [root]
        elif root.is_dir():
            candidates = []
            for directory, dirs, names in os.walk(root, followlinks=False):
                dirs[:] = [d for d in dirs if d not in SKIP_DIRS and not (root in NATIVE_RUNTIME and d in ('tmp','debug','log','logs'))]
                candidates.extend(Path(directory) / n for n in names)
                candidates.extend(Path(directory) / d for d in dirs if (Path(directory)/d).is_symlink())
        else: continue
        for path in candidates:
            relative = str(path.relative_to(HOME))
            if relative not in seen:
                seen.add(relative)
                yield relative, path


def roots():
    result = [HOME/'projects', DATA, *[HOME/p for p in NATIVE], *[HOME/p for p in ROOTS]]
    if (DATA/'state.db').exists():
        with sqlite3.connect(f'file:{DATA}/state.db?mode=ro', uri=True) as db:
            for (value,) in db.execute('SELECT path FROM project'):
                path = Path(value)
                if path.is_absolute():
                    if not path.is_relative_to(HOME): raise ValueError('Move local folders inside /home/agent before backing up this machine')
                    result.append(path)
    if CONFIG.exists():
        config = tomlkit.parse(CONFIG.read_text()).unwrap()
        for profile in config.get('profiles', []):
            for key in ('CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'OPENCODE_DATA_DIR', 'HF_HOME'):
                value = profile.get('env', {}).get(key)
                if value:
                    path = Path(value.replace('~', str(HOME), 1))
                    if not path.is_absolute() or not path.is_relative_to(HOME):
                        raise ValueError('Profile credential roots must remain inside the agent home')
                    result.append(path)
                    NATIVE_RUNTIME.add(path)
    classify_git(result)
    return result


def is_sqlite(path):
    with path.open('rb') as f: return f.read(16) == b'SQLite format 3\x00'


def credential_bytes(path):
    if not is_sqlite(path): return path.read_bytes()
    with tempfile.TemporaryDirectory() as directory:
        copy = Path(directory)/'credential.db'
        sqlite_snapshot(path, copy)
        return copy.read_bytes()


def capture(approved, release, clean=False):
    secrets = {'references': {}, 'files': {}, 'present': []}
    ordinary = {}
    with tempfile.TemporaryDirectory() as directory:
        state = Path(directory)/'state.db'
        if (DATA/'state.db').exists():
            materialize(DATA/'state.db', state, lambda db: project_db(db, approved, secrets['references']))
            ordinary['state/state.db'] = state.read_bytes()
        if (DATA/'cache.db').exists():
            cache = Path(directory)/'cache.db'
            sqlite_snapshot(DATA/'cache.db', cache)
            ordinary['state/cache.db'] = cache.read_bytes()
        if CONFIG.exists(): ordinary['config/config.toml'] = config_split(CONFIG.read_text(), secrets['references'])
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode='w:gz', dereference=False) as tar:
            for name, path in files_under(roots()):
                if clean and '.git' in Path(name).parts and (path.name.endswith('.lock') or any(part in ('MERGE_HEAD','CHERRY_PICK_HEAD','rebase-merge','rebase-apply') for part in Path(name).parts)):
                    raise ValueError('A Git operation is in progress; finish it before maintenance')
                if path.parent == DATA and path.name in EXCLUDED: continue
                if path == CONFIG: continue
                if path.is_symlink():
                    if not path.resolve().is_relative_to(HOME):
                        raise ValueError('A workspace symlink leaves the backed-up home; checkpoint paused')
                if credential(name):
                    if path.is_symlink(): raise ValueError('Credential symlinks need a registered local file')
                    secrets['files'][name] = base64.b64encode(credential_bytes(path)).decode()
                    secrets['present'].append(name)
                elif path.is_file() or path.is_symlink():
                    # Tool-native SQLite files are snapshotted through the online API too.
                    if path.is_file() and is_sqlite(path):
                        native = Path(directory)/('native-'+uuid.uuid4().hex)
                        sqlite_snapshot(path, native)
                        info = tarfile.TarInfo(name); info.size = native.stat().st_size; info.mode = 0o600
                        with native.open('rb') as f: tar.addfile(info, f)
                    elif not name.endswith(('-wal', '-shm')):
                        if path.is_symlink(): tar.add(path,arcname=name,recursive=False)
                        else:
                            info=tarfile.TarInfo(name); stat=path.stat();info.size=stat.st_size;info.mode=stat.st_mode & 0o777;info.mtime=stat.st_mtime
                            with path.open('rb') as f: tar.addfile(info,f)
        ordinary['workspaces/home.tar.gz'] = archive.getvalue()
    ordinary['state/runtime.json'] = json.dumps({'format':1,'release':release,'at':time.time()}).encode()
    return ordinary, secrets


def credential_snapshot():
    from base64 import b64encode
    files = {}
    for name, path in files_under(roots()):
        if credential(name) and not path.is_symlink(): files[name] = b64encode(credential_bytes(path)).decode()
    # Config also holds literal API keys; never upload its bytes outside the vault.
    if CONFIG.exists(): files['.config/launchpad/config.toml'] = b64encode(CONFIG.read_bytes()).decode()
    return files


def unpack(data):
    # Validate ALL members before writing; never use extractall.
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tar:
        members = tar.getmembers()
        for member in members:
            safe_name(member.name)
            destination = HOME/member.name
            if not destination.resolve().is_relative_to(HOME): raise ValueError('Unsafe archive destination')
            if member.issym():
                target = destination.parent/member.linkname
                if not target.resolve().is_relative_to(HOME): raise ValueError('Unsafe archive symlink')
            elif not member.isfile(): raise ValueError('Unsupported archive member')
        for member in members:
            if member.isfile():
                atomic(HOME/member.name, tar.extractfile(member).read())
                os.chmod(HOME/member.name,0o700 if member.mode & 0o111 else 0o600)
        for member in members:
            if member.issym():
                target = HOME/member.name
                target.parent.mkdir(parents=True, exist_ok=True)
                for parent in [target.parent,*target.parent.parents]:
                    if parent == HOME: break
                    own(parent,True)
                if target.exists() or target.is_symlink(): target.unlink()
                target.symlink_to(member.linkname)
                os.lchown(target, pwd.getpwnam('agent').pw_uid, pwd.getpwnam('agent').pw_gid)


def restore(files, payload, lost_key=False):
    unpack(files['workspaces/home.tar.gz'])
    for name in ('state.db', 'cache.db'):
        if 'state/'+name in files:
            atomic(DATA/name, files['state/'+name])
            for suffix in ('-wal', '-shm'): (DATA/(name+suffix)).unlink(missing_ok=True)
    if 'config/config.toml' in files:
        if lost_key:
            config = tomlkit.parse(files['config/config.toml'].decode()).unwrap()
            for profile in config.get('profiles', []): profile['env'] = {}
            atomic(CONFIG, tomlkit.dumps(config).encode())
        else: atomic(CONFIG, config_join(files['config/config.toml'], payload['references']))
    if (DATA/'state.db').exists(): restore_db(DATA/'state.db', payload.get('references', {}), lost_key)
    if not lost_key:
        from base64 import b64decode
        for name, data in payload['files'].items():
            safe_name(name)
            if not credential(name) and '.git' not in Path(name).parts: raise ValueError('Unregistered credential destination')
            atomic(HOME/name, b64decode(data, validate=True))
    # Browser credentials and PTYs deliberately do not survive a machine restore.
