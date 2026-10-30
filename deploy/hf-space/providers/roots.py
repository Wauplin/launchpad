"""Version 1 credential registry. Paths are relative to the stable agent home."""
from pathlib import PurePosixPath

ROOTS = ('.codex/auth.json', '.codex/config.toml', '.claude/settings.json', '.claude/settings.local.json', '.config/opencode', '.claude/.credentials.json', '.claude.json',
         '.local/share/opencode/auth.json', '.config/gh', '.cache/huggingface/token',
         '.cache/huggingface/stored_tokens', '.ssh', '.git-credentials', '.gitconfig')
NATIVE = ('.codex', '.claude', '.local/share/opencode', '.config/opencode')


def credential(name):
    parts = PurePosixPath(name).parts
    # Known workspace secret files, including copies in registered profile roots.
    if any(p == '.env' or p.startswith('.env.') or p in ('auth.json', '.credentials.json', 'id_ed25519', 'id_rsa') for p in parts): return True
    if any(name == p or name.startswith(p+'/') for p in PROTECTED_GIT): return True
    if '.git' in parts and ('logs' in parts or parts[-1]=='config'): return True
    if PurePosixPath(name).name in ('config.toml','settings.json','settings.local.json'): return True
    if name.endswith('/.git/config'): return True
    if name.endswith(('.pem', '.key', '.p12', '.pfx')): return True
    return any(name == root or name.startswith(root + '/') for root in ROOTS)


PROTECTED_GIT = set()


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
            prefix=str((__import__('pathlib').Path(directory)/'.git').relative_to(HOME))
            if prefix in seen: continue
            seen.add(prefix)
            result=subprocess.run(['git','--no-pager','-C',directory,'log','--all','--format=','--name-only','--',
                '.env','.env.*','*.pem','*.key','*auth.json','*credentials*'],env=environment(),preexec_fn=demote,
                stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=20)
            if result.stdout.strip(): PROTECTED_GIT.add(prefix)
            elif result.returncode==0: PROTECTED_GIT.discard(prefix)
            elif not any(p.is_file() for p in (__import__('pathlib').Path(directory)/'.git/objects').rglob('*')):
                pass # An empty repository has no history to classify.
            else: raise ValueError('Git history could not be classified; recovery upload paused')
