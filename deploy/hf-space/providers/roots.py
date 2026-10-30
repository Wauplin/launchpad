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
    if '.git' in parts: return True
    if PurePosixPath(name).name in ('config.toml','settings.json','settings.local.json'): return True
    if name.endswith('/.git/config'): return True
    if name.endswith(('.pem', '.key', '.p12', '.pfx')): return True
    return any(name == root or name.startswith(root + '/') for root in ROOTS)
