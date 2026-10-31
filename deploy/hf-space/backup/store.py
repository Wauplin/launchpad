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

    def copy(self, source, destination):
        # Never include command output: auth failures can contain sensitive metadata.
        result = subprocess.run(["hf", "buckets", "cp", str(source), str(destination), "--quiet"],
                                env=self.environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
        if result.returncode:
            raise RuntimeError("Bucket transfer failed; check access and available capacity")

    def exists(self, name):
        from huggingface_hub import HfApi
        return bool(list(HfApi(token=self.environment['HF_TOKEN']).get_bucket_paths_info(self.bucket_id, [name])))

    def read(self, name):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "object"
            self.copy(f"{self.root}/{name}", path)
            return path.read_bytes()

    def put(self, name, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "object"
            path.write_bytes(data)
            self.copy(path, f"{self.root}/{name}")
        if self.read(name) != data:
            raise RuntimeError("Bucket read-back did not match; checkpoint was not committed")

    def commit(self, files, vault, payload, installation, release, captured_at=0):
        generation = uuid.uuid4().hex
        sealed = vault.seal(generation, payload)
        if vault.open(generation, sealed) != payload:
            raise RuntimeError("Credential verification failed")
        self.put(f"credentials/{generation}/vault.aead", sealed)
        entries = {}
        for name, data in files.items():
            safe_name(name)
            self.put(f"backups/{generation}/{name}", data)
            entries[name] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
        manifest = {"format": 1, "installation": installation, "generation": generation,
                    "release": release, "captured_at":captured_at, "credentials": generation, "files": entries, "bytes":len(sealed)+sum(len(data) for data in files.values())}
        data = json.dumps(manifest, sort_keys=True).encode()
        self.put(f"backups/{generation}/manifest.json", data)
        self.put("backups/latest.json", json.dumps({"generation": generation, "sha256": hashlib.sha256(data).hexdigest()}).encode())
        return manifest

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
            data = self.read(f"backups/{generation}/{name}")
            if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
                raise ValueError("Checkpoint checksum mismatch; restore aborted")
            files[name] = data
        return manifest, files


def safe_name(name):
    if not name or name.startswith("/") or "\\" in name or any(p in ("", ".", "..") for p in name.split("/")):
        raise ValueError("Invalid recovery path")
