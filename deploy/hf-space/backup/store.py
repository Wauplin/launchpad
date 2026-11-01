"""Read-back verified bucket objects. Only the latest state is kept: `space/manifest.json` and the objects it names.

Changed objects alternate between two slots (`space/a/`, `space/b/`), so the committed manifest always points at
complete objects while the next ones upload; nothing accumulates. `projects/` is the live Space volume and is never
written here. `backups/` and `credentials/` are the read-only format 1 layout, used once to migrate.
"""
import hashlib
import json
import subprocess
import tempfile
import uuid
from pathlib import Path

ROOT = "space"
MANIFEST = f"{ROOT}/manifest.json"
CREDENTIALS = f"{ROOT}/credentials.aead"
PRE_UPDATE = f"{ROOT}/pre-update"
UPDATE = f"{ROOT}/update.json"
# Credential copies under a new key, written by a planned rotation before the Space secret changes.
NEXT = f"{ROOT}/next"


class Bucket:
    def __init__(self, bucket, environment):
        self.root = f"hf://buckets/{bucket}"
        self.environment = environment
        self.bucket_id = bucket

    def copy(self, source, destination, size=0):
        # Never include command output: auth failures can contain sensitive metadata.
        # Large objects get more time: at least 0.5 MB/s on top of a fixed allowance.
        result = subprocess.run(["hf", "buckets", "cp", str(source), str(destination), "--quiet"],
                                env=self.environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                timeout=120 + size // (512 * 1024))
        if result.returncode:
            raise RuntimeError("Bucket transfer failed; check access and available capacity")

    def exists(self, name):
        from huggingface_hub import HfApi
        return bool(list(HfApi(token=self.environment['HF_TOKEN']).get_bucket_paths_info(self.bucket_id, [name])))

    def read(self, name, size=0):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "object"
            self.copy(f"{self.root}/{name}", path, size)
            return path.read_bytes()

    def read_json(self, name):
        return json.loads(self.read(name)) if self.exists(name) else None

    def put(self, name, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "object"
            path.write_bytes(data)
            self.copy(path, f"{self.root}/{name}", len(data))
        if self.read(name, len(data)) != data:
            raise RuntimeError("Bucket read-back did not match; checkpoint was not committed")

    def commit(self, files, installation, release, captured_at=0, previous=None):
        """Upload changed objects to the free slot, then replace the manifest. Unchanged state writes nothing.

        Files missing from `files` (the agent archive, captured less often) are carried over from `previous`.
        """
        entries, uploads = plan(files, previous)
        if previous and entries == previous.get("files"):
            return previous
        for name in uploads:
            self.put(entries[name]["path"], files[name])
        manifest = {"format": 2, "installation": installation, "checkpoint": uuid.uuid4().hex, "release": release,
                    "captured_at": captured_at, "files": entries, "bytes": sum(e["bytes"] for e in entries.values())}
        self.put(MANIFEST, json.dumps(manifest, sort_keys=True).encode())
        return manifest

    def latest(self):
        manifest = self.read_json(MANIFEST)
        if manifest is not None and manifest.get("format") != 2:
            raise ValueError("Unsupported checkpoint format")
        return manifest

    def restore(self, manifest):
        files = {}
        for name, entry in manifest["files"].items():
            data = self.read(object_path(name, entry), entry["bytes"])
            if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("Checkpoint checksum mismatch; restore aborted")
            files[name] = data
        return files

    def save_pre_update(self, manifest, files):
        """One fixed restore point for update rollback, replaced by the next update."""
        entries = {}
        for name, entry in manifest["files"].items():
            data = files[name]
            if hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("The pre-update checkpoint changed; try the update again")
            entries[name] = dict(entry, path=f"{PRE_UPDATE}/{name}")
            self.put(entries[name]["path"], data)
        self.put(f"{PRE_UPDATE}/manifest.json", json.dumps(dict(manifest, files=entries), sort_keys=True).encode())

    def pre_update(self, checkpoint):
        manifest = self.read_json(f"{PRE_UPDATE}/manifest.json")
        if not manifest or manifest.get("format") != 2 or manifest.get("checkpoint") != checkpoint:
            raise ValueError("The requested pre-update checkpoint is not in the bucket")
        return manifest

    def legacy(self, generation=None):
        """Format 1 (`backups/<generation>/`), read only to migrate an existing Space. None when absent."""
        if generation is None:
            pointer = self.read_json("backups/latest.json")
            if pointer is None:
                return None
            generation = pointer["generation"]
        safe_name(generation)
        if "/" in generation:
            raise ValueError("Invalid generation")
        manifest = json.loads(self.read(f"backups/{generation}/manifest.json"))
        if manifest.get("format") != 1 or manifest.get("generation") != generation:
            raise ValueError("Unsupported or incomplete checkpoint")
        files = {}
        for name, entry in manifest["files"].items():
            safe_name(name)
            path = entry.get("path") or f"backups/{generation}/{name}"
            safe_name(path)
            if not path.startswith("backups/") or not path.endswith("/" + name):
                raise ValueError("Invalid checkpoint object path")
            data = self.read(path, entry["bytes"])
            if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("Checkpoint checksum mismatch; restore aborted")
            files[name] = data
        return manifest, files


def plan(files, previous):
    """Manifest entries for `files` and the names to upload. A changed object goes to the slot its previous
    version does not use, so the committed manifest never points at an object being overwritten."""
    old = (previous or {}).get("files", {})
    entries, uploads = {}, []
    for name, data in files.items():
        safe_name(name)
        digest = hashlib.sha256(data).hexdigest()
        entry = old.get(name)
        if entry and entry["sha256"] == digest and entry["bytes"] == len(data):
            entries[name] = entry
            continue
        slot = "b" if entry and entry.get("path", "").startswith(f"{ROOT}/a/") else "a"
        entries[name] = {"sha256": digest, "bytes": len(data), "path": f"{ROOT}/{slot}/{name}"}
        uploads.append(name)
    for name, entry in old.items():
        entries.setdefault(name, entry)
    return entries, uploads


def object_path(name, entry):
    path = entry.get("path", "")
    safe_name(name)
    safe_name(path)
    if not path.startswith(ROOT + "/") or not path.endswith("/" + name):
        raise ValueError("Invalid checkpoint object path")
    return path


def safe_name(name):
    if not name or name.startswith("/") or "\\" in name or any(p in ("", ".", "..") for p in name.split("/")):
        raise ValueError("Invalid recovery path")
