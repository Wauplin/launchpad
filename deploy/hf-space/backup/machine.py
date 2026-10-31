"""Recoverable local files plus separately sealed credentials, at stable paths."""
import base64
import pwd
import io
import json
import os
import sqlite3
import stat
import tarfile
import tempfile
import time
import uuid
from pathlib import Path

import tomlkit

from backup.projection import config_split, config_join, project_db, restore_db, REF
from backup.snapshot import materialize, sqlite_snapshot
from backup.store import safe_name
from providers.roots import credential, NATIVE, ROOTS, classify_git, CONFIG_ROOTS
from supervisor.processes import HOME, own

DATA = HOME / '.local/share/launchpad'
CONFIG = HOME / '.config/launchpad/config.toml'
EXCLUDED = {'token', 'vapid.json', 'state.db', 'state.db-wal', 'state.db-shm', 'cache.db', 'cache.db-wal', 'cache.db-shm'}
NATIVE_RUNTIME = {HOME/name for name in NATIVE}
SKIP_DIRS = {'node_modules', 'target', '__pycache__', '.venv', '.cache', '.npm'}


def secure_parent(path):
    """Open owned directory descriptors without following agent-controlled symlinks."""
    parts=Path(path).relative_to(HOME).parts
    if not parts or any(part in ('.','..') for part in parts): raise ValueError('Invalid agent path')
    user=pwd.getpwnam('agent')
    flags=os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    current=os.open(HOME,flags)
    try:
        for part in parts[:-1]:
            try: child=os.open(part,flags,dir_fd=current)
            except FileNotFoundError:
                os.mkdir(part,0o700,dir_fd=current)
                child=os.open(part,flags,dir_fd=current)
            os.fchown(child,user.pw_uid,user.pw_gid)
            os.fchmod(child,0o700)
            os.close(current)
            current=child
        return current,parts[-1]
    except BaseException:
        os.close(current)
        raise


def home_open(path):
    """Read agent files through no-follow descriptors, including every parent."""
    parts=Path(path).relative_to(HOME).parts
    if not parts or any(part in ('.','..') for part in parts): raise ValueError('Invalid agent path')
    flags=os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    current=os.open(HOME,flags)
    try:
        for part in parts[:-1]:
            child=os.open(part,flags,dir_fd=current)
            os.close(current)
            current=child
        fd=os.open(parts[-1],os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,dir_fd=current)
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            os.close(fd)
            raise ValueError('Only regular agent files can be read')
        return os.fdopen(fd,'rb')
    finally: os.close(current)


def atomic(path, data):
    parent,name=secure_parent(path)
    temporary='.lp-restore-'+uuid.uuid4().hex
    try:
        fd=os.open(temporary,os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,0o600,dir_fd=parent)
        user=pwd.getpwnam('agent')
        with os.fdopen(fd,'wb') as f:
            os.fchown(f.fileno(),user.pw_uid,user.pw_gid)
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        # Replacing a leaf symlink replaces the link itself, never its target.
        os.replace(temporary,name,src_dir_fd=parent,dst_dir_fd=parent)
        os.fsync(parent)
    finally:
        try: os.unlink(temporary,dir_fd=parent)
        except FileNotFoundError: pass
        os.close(parent)


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
            if relative == '.agents/skills/launchpad' and path.is_symlink() and path.resolve() == Path('/src/skills/launchpad'): continue
            if not path.resolve().is_relative_to(HOME): raise ValueError('A source path leaves the registered agent home')
            mode=path.lstat().st_mode
            if not (stat.S_ISREG(mode) or stat.S_ISLNK(mode)): continue
            if relative not in seen:
                seen.add(relative)
                yield relative, path


def roots():
    result = [HOME/'projects', HOME/'.agents', DATA, *[HOME/p for p in NATIVE], *[HOME/p for p in ROOTS]]
    if (DATA/'state.db').exists():
        with sqlite3.connect(f'file:{DATA}/state.db?mode=ro', uri=True) as db:
            for (value,) in db.execute('SELECT path FROM project'):
                path = Path(value)
                if path.is_absolute():
                    if path == HOME: raise ValueError('Register a dedicated folder rather than the whole agent home')
                    if not path.is_relative_to(HOME): raise ValueError('Move local folders inside /home/agent before backing up this machine')
                    result.append(path)
    if CONFIG.exists():
        config = tomlkit.parse(home_open(CONFIG).read().decode()).unwrap()
        for profile in config.get('profiles', []):
            for key in ('CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'OPENCODE_DATA_DIR', 'HF_HOME'):
                value = profile.get('env', {}).get(key)
                if value:
                    path = Path(value.replace('${HOME}',str(HOME)).replace('~', str(HOME), 1))
                    if path == HOME: raise ValueError('Use a dedicated provider folder rather than the whole agent home')
                    if not path.is_absolute() or not path.is_relative_to(HOME):
                        raise ValueError('Profile credential roots must remain inside the agent home')
                    result.append(path)
                    NATIVE_RUNTIME.add(path)
                    CONFIG_ROOTS.add(str(path.relative_to(HOME)))
    classify_git(result)
    return result


def is_sqlite(path):
    with home_open(path) as f: return f.read(16) == b'SQLite format 3\x00'


def signature(path):
    value=path.stat()
    return value.st_ino,value.st_size,value.st_mtime_ns


def credential_bytes(path):
    before=signature(path)
    if not is_sqlite(path):
        data=home_open(path).read()
        if signature(path)!=before: raise ValueError('Credentials changed while being captured; retry shortly')
        if path.suffix=='.json': json.loads(data)
        if path.name=='config.toml': tomlkit.parse(data.decode())
        return data
    with tempfile.TemporaryDirectory() as directory:
        copy = Path(directory)/'credential.db'
        sqlite_snapshot(path, copy)
        return copy.read_bytes()


def capture(approved, release, clean=False):
    secrets = {'references': {}, 'files': {}, 'present': []}
    credential_stats={}
    sources=roots()
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
        if CONFIG.exists(): ordinary['config/config.toml'] = config_split(home_open(CONFIG).read().decode(), secrets['references'])
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode='w:gz', dereference=False) as tar:
            for name, path in files_under(sources):
                if clean and '.git' in Path(name).parts and (path.name.endswith('.lock') or any(part in ('MERGE_HEAD','CHERRY_PICK_HEAD','rebase-merge','rebase-apply') for part in Path(name).parts)):
                    raise ValueError('A Git operation is in progress; finish it before maintenance')
                if path.parent == DATA and path.name in EXCLUDED: continue
                if path == CONFIG: continue
                if path.is_symlink():
                    if not path.resolve().is_relative_to(HOME):
                        raise ValueError('A workspace symlink leaves the backed-up home; checkpoint paused')
                if credential(name):
                    if path.is_symlink(): raise ValueError('Credential symlinks need a registered local file')
                    credential_stats[path]=signature(path)
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
                            with home_open(path) as f: tar.addfile(info,f)
        if any(signature(path)!=value for path,value in credential_stats.items()): raise ValueError('Credentials changed during checkpoint; retry shortly')
        ordinary['workspaces/home.tar.gz'] = archive.getvalue()
    ordinary['state/runtime.json'] = json.dumps({'format':1,'release':release,'at':time.time(),'provider_directories':sorted(CONFIG_ROOTS),'directories':[str(path.relative_to(HOME)) for path in sources if path.is_dir() and path != HOME]}).encode()
    return ordinary, secrets


def credential_snapshot():
    from base64 import b64encode
    files = {}
    signatures={}
    for name, path in files_under(roots()):
        if credential(name) and not path.is_symlink():
            signatures[path]=signature(path)
            files[name] = b64encode(credential_bytes(path)).decode()
    # Config also holds literal API keys; never upload its bytes outside the vault.
    if CONFIG.exists(): files['.config/launchpad/config.toml'] = b64encode(home_open(CONFIG).read()).decode()
    if any(signature(path)!=value for path,value in signatures.items()): raise ValueError('Credentials changed during capture; retry shortly')
    return files


def unpack(data, directories):
    # Validate ALL members before writing; never use extractall.
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tar:
        members = tar.getmembers()
        for member in members:
            safe_name(member.name)
            if not any(member.name.startswith(root+'/') for root in directories): raise ValueError('Unregistered archive destination')
            if credential(member.name): raise ValueError('Credential file found in an ordinary archive')
            if Path(member.name).parent == DATA.relative_to(HOME) and Path(member.name).name in EXCLUDED: raise ValueError('Control state found in an ordinary archive')
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
                parent,name=secure_parent(target)
                try:
                    try: os.unlink(name,dir_fd=parent)
                    except FileNotFoundError: pass
                    os.symlink(member.linkname,name,dir_fd=parent)
                    user=pwd.getpwnam('agent')
                    os.chown(name,user.pw_uid,user.pw_gid,dir_fd=parent,follow_symlinks=False)
                finally: os.close(parent)
        for member in members:
            if member.issym() and not (HOME/member.name).resolve().is_relative_to(HOME):
                raise ValueError('Archive symlink chain leaves the registered home')


def restore(files, payload, lost_key=False):
    metadata=json.loads(files['state/runtime.json'])
    for directory in metadata.get('provider_directories',[]):
        safe_name(directory)
        CONFIG_ROOTS.add(directory)
    directories=metadata['directories']
    for directory in directories: safe_name(directory)
    unpack(files['workspaces/home.tar.gz'],directories)
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
