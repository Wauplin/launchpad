"""Version 1 credential registry. Paths are relative to the stable agent home."""
from pathlib import Path, PurePosixPath

ROOTS = ('.codex/auth.json', '.codex/config.toml', '.claude/settings.json', '.claude/settings.local.json', '.config/opencode', '.claude/.credentials.json', '.claude.json',
         '.local/share/opencode/auth.json', '.config/gh', '.cache/huggingface/token',
         '.cache/huggingface/stored_tokens', '.ssh', '.git-credentials', '.gitconfig', '.config/git')
NATIVE = ('.codex', '.claude', '.local/share/opencode', '.config/opencode')
CONFIG_ROOTS = set(NATIVE)


def credential(name):
    parts = PurePosixPath(name).parts
    # Known workspace secret files, including copies in registered profile roots.
    if any(p == '.env' or p.startswith('.env.') or p in ('auth.json', '.credentials.json', 'id_ed25519', 'id_rsa', '.git-credentials', 'stored_tokens') for p in parts): return True
    if any(name == p or name.startswith(p+'/') for p in PROTECTED_GIT): return True
    if '.git' in parts and ('logs' in parts or parts[-1]=='config'): return True
    if PurePosixPath(name).name in ('config.toml','settings.json','settings.local.json') and any(name.startswith(root+'/') for root in CONFIG_ROOTS): return True
    if name.endswith('/.git/config'): return True
    if name.endswith(('.pem', '.key', '.p12', '.pfx')): return True
    return any(name == root or name.startswith(root + '/') for root in ROOTS)


PROTECTED_GIT = set()
# Repository -> refs signature of its last classification; history only changes when refs move.
_classified = {}


def refs_signature(git):
    """Cheap change key for a repository's history: mtimes of HEAD, packed refs, reflog and ref folders."""
    import os
    values=[]
    for name in ('HEAD','packed-refs','logs/HEAD','FETCH_HEAD'):
        try: values.append((name,os.stat(git/name).st_mtime_ns))
        except OSError: pass
    for directory,_,_ in os.walk(git/'refs'):
        try: values.append((directory,os.stat(directory).st_mtime_ns))
        except OSError: pass
    return tuple(values)


def classify_git(paths):
    """Seal Git histories containing registered credential filenames, preserve ordinary Git."""
    import os
    import subprocess
    from supervisor.processes import HOME, environment, demote
    seen=set()
    for root in paths:
        if not root.is_dir(): continue
        for directory, dirs, _ in os.walk(root, followlinks=False):
            dirs[:]=[d for d in dirs if d not in ('node_modules','target','.venv','.cache','__pycache__')]
            if '.git' not in dirs: continue
            dirs.remove('.git')
            prefix=str((Path(directory)/'.git').relative_to(HOME))
            if prefix in seen: continue
            seen.add(prefix)
            relative=str(Path(directory).relative_to(HOME))
            key=refs_signature(Path(directory)/'.git')
            if _classified.get(prefix, (None,))[0] == key: continue
            # Templates such as .env.example are meant to be committed and hold no secrets.
            patterns=['.env','.env.*',':!*.example',':!*.sample',':!*.template',':!*.dist','*.pem','*.key','*.p12','*.pfx',
                      '*auth.json','*credentials.json','.credentials*','credentials','id_ed25519','id_rsa','stored_tokens']
            if any(relative==root or relative.startswith(root+'/') or root.startswith(relative+'/') for root in CONFIG_ROOTS): patterns += ['config.toml','settings.json','settings.local.json']
            result=subprocess.run(['git','--no-pager','-C',directory,'log','--all','--format=','--name-only','--',
                *patterns],env=environment(),preexec_fn=demote,
                stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=20)
            if result.stdout.strip(): PROTECTED_GIT.add(prefix)
            elif result.returncode==0: PROTECTED_GIT.discard(prefix)
            if result.returncode==0: _classified[prefix]=(key,)
            elif not any(p.is_file() for p in (Path(directory)/'.git/objects').rglob('*')):
                pass # An empty repository has no history to classify.
            else: raise ValueError('Git history could not be classified; recovery upload paused')
