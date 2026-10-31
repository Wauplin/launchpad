"""Immutable, read-back verified bucket commits. latest.json is only a hint."""
import hashlib
import json
import subprocess
import tempfile
import uuid
from pathlib import Path


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

    def put(self, name, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "object"
            path.write_bytes(data)
            self.copy(path, f"{self.root}/{name}", len(data))
        if self.read(name, len(data)) != data:
            raise RuntimeError("Bucket read-back did not match; checkpoint was not committed")

    def commit(self, files, vault, payload, installation, release, captured_at=0, previous=None, credentials=None):
        """Write one generation: changed objects, then the manifest, then latest.json.

        `previous` is the last committed manifest: unchanged objects are referenced from it instead of being
        uploaded again, and files missing from `files` (a workspace captured less often) are carried over.
        `credentials` names an existing vault generation holding exactly `payload`; otherwise a new one is
        sealed. Returns `previous` itself when nothing changed, so an idle machine writes nothing.
        """
        generation = uuid.uuid4().hex
        old = (previous or {}).get("files", {})
        entries = {}
        for name, data in files.items():
            safe_name(name)
            digest = hashlib.sha256(data).hexdigest()
            entry = old.get(name)
            if entry and entry["sha256"] == digest and entry["bytes"] == len(data):
                entries[name] = dict(entry, path=object_path(previous["generation"], name, entry))
            else:
                entries[name] = {"sha256": digest, "bytes": len(data), "path": f"backups/{generation}/{name}"}
        for name, entry in old.items():
            if name not in entries:
                entries[name] = dict(entry, path=object_path(previous["generation"], name, entry))
        if previous and credentials == previous.get("credentials") and entries == {
                name: dict(entry, path=object_path(previous["generation"], name, entry)) for name, entry in old.items()}:
            return previous
        if credentials is None:
            credentials = generation
            sealed = vault.seal(generation, payload)
            if vault.open(generation, sealed) != payload:
                raise RuntimeError("Credential verification failed")
            self.put(f"credentials/{generation}/vault.aead", sealed)
        for name, data in files.items():
            if entries[name]["path"] == f"backups/{generation}/{name}":
                self.put(entries[name]["path"], data)
        manifest = {"format": 1, "installation": installation, "generation": generation,
                    "release": release, "captured_at": captured_at, "credentials": credentials, "files": entries,
                    "bytes": sum(entry["bytes"] for entry in entries.values())}
        self.put_manifest(manifest)
        return manifest

    def put_manifest(self, manifest):
        data = json.dumps(manifest, sort_keys=True).encode()
        self.put(f"backups/{manifest['generation']}/manifest.json", data)
        self.put("backups/latest.json", json.dumps({"generation": manifest["generation"], "sha256": hashlib.sha256(data).hexdigest()}).encode())

    def restore(self, generation):
        safe_name(generation)
        if "/" in generation:
            raise ValueError("Invalid generation")
        manifest = json.loads(self.read(f"backups/{generation}/manifest.json"))
        if manifest.get("format") != 1 or manifest.get("generation") != generation:
            raise ValueError("Unsupported or incomplete checkpoint")
        files = {}
        for name, entry in manifest["files"].items():
            safe_name(name)
            data = self.read(object_path(generation, name, entry), entry["bytes"])
            if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("Checkpoint checksum mismatch; restore aborted")
            files[name] = data
        return manifest, files


def object_path(generation, name, entry):
    """Where a manifest entry's bytes live: its own generation, or an earlier one it references."""
    path = entry.get("path") or f"backups/{generation}/{name}"
    safe_name(path)
    if not path.startswith("backups/") or not path.endswith("/" + name):
        raise ValueError("Invalid checkpoint object path")
    return path


def safe_name(name):
    if not name or name.startswith("/") or "\\" in name or any(p in ("", ".", "..") for p in name.split("/")):
        raise ValueError("Invalid recovery path")
