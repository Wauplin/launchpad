"""Schema-bound credential splitting. Trace bodies remain intentionally readable."""
import hashlib
import json
import re
import sqlite3
from pathlib import Path

import tomlkit

SENSITIVE = re.compile(r'(token|password|secret|api.?key|credential|authorization|private.?key)', re.I)
REF = '__lp_space_credential__'


def fingerprint(db):
    schema = db.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").fetchall()
    return hashlib.sha256(json.dumps(schema).encode()).hexdigest()


def split(value, secrets):
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if ((SENSITIVE.search(key) and isinstance(item, str)) or (key == 'env' and isinstance(item, dict))) and item not in (None, '', {}, []):
                reference = str(len(secrets))
                secrets[reference] = item
                out[key] = {REF: reference}
            else:
                out[key] = split(item, secrets)
        return out
    if isinstance(value, list): return [split(item, secrets) for item in value]
    return value


def join(value, secrets, missing=False):
    if isinstance(value, dict):
        if set(value) == {REF}:
            key = value[REF]
            if key not in secrets:
                if missing: return {} # only env maps are restored this way by config recovery
                raise ValueError('Missing credential reference; checkpoint preserved')
            return secrets[key]
        return {k: join(v, secrets, missing) for k, v in value.items()}
    if isinstance(value, list): return [join(v, secrets, missing) for v in value]
    return value


def config_split(data, secrets):
    return tomlkit.dumps(split(tomlkit.parse(data).unwrap(), secrets)).encode()


def config_join(data, secrets):
    return tomlkit.dumps(join(tomlkit.parse(data.decode()).unwrap(), secrets)).encode()


def project_db(db, approved, secrets):
    if fingerprint(db) != approved:
        raise ValueError('State schema is not classified by this image; recovery upload paused')
    for table in ('browser_device', 'push_subscription'):
        db.execute(f'DELETE FROM "{table}"')
    # Walk every JSON-valued TEXT column, including native launch specs and proposals.
    # Freeform prompts, tool output and traces have the report\'s explicit plaintext tradeoff.
    for (table,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").fetchall():
        quote = lambda name: '"' + name.replace('"', '""') + '"'
        columns = [row[1] for row in db.execute(f'PRAGMA table_info({quote(table)})') if 'TEXT' in row[2].upper()]
        if table in ('browser_device','push_subscription'): continue
        for column in columns:
            if SENSITIVE.search(column): raise ValueError('A raw credential column needs an adapter projection before this schema can be backed up')
            for rowid, value in db.execute(f'SELECT rowid,{quote(column)} FROM {quote(table)} WHERE {quote(column)} IS NOT NULL').fetchall():
                if not isinstance(value, str) or not value.lstrip().startswith(('{', '[')): continue
                try: parsed = json.loads(value)
                except ValueError: continue
                projected = split(parsed, secrets)
                if projected != parsed:
                    db.execute(f'UPDATE {quote(table)} SET {quote(column)}=? WHERE rowid=?', (json.dumps(projected), rowid))


def restore_db(path, secrets, missing=False):
    with sqlite3.connect(path) as db:
        for (table,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").fetchall():
            q = lambda name: '"' + name.replace('"', '""') + '"'
            for column in [r[1] for r in db.execute(f'PRAGMA table_info({q(table)})') if 'TEXT' in r[2].upper()]:
                for rowid, value in db.execute(f'SELECT rowid,{q(column)} FROM {q(table)}').fetchall():
                    if isinstance(value, str) and REF in value:
                        db.execute(f'UPDATE {q(table)} SET {q(column)}=? WHERE rowid=?', (json.dumps(join(json.loads(value), secrets, missing)), rowid))


if __name__ == '__main__':
    import sys
    with sqlite3.connect(sys.argv[1]) as db:
        Path(sys.argv[2]).write_text(json.dumps({'state_schema': fingerprint(db)}))
