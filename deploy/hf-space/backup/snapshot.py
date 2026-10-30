"""Local SQLite snapshots. Never copy a live WAL database as a plain file."""
import sqlite3
from pathlib import Path


def sqlite_snapshot(source, destination):
    source, destination = Path(source), Path(destination)
    with sqlite3.connect(source.resolve().as_uri() + "?mode=ro", uri=True, timeout=5) as live:
        with sqlite3.connect(destination) as snapshot:
            live.backup(snapshot)
            if snapshot.execute("PRAGMA integrity_check").fetchone() != ("ok",):
                raise ValueError("Snapshot integrity check failed")


def materialize(source, destination, project):
    """Project a copied DB, then VACUUM out deleted bytes before any upload.

    The caller MUST supply a schema-aware projector; unknown schemas must raise.
    No catch-all raw DB upload fallback is permitted.
    """
    sqlite_snapshot(source, destination)
    with sqlite3.connect(destination) as snapshot:
        project(snapshot)
        snapshot.commit()
        snapshot.execute("VACUUM")
        if snapshot.execute("PRAGMA integrity_check").fetchone() != ("ok",):
            raise ValueError("Projected snapshot integrity check failed")
