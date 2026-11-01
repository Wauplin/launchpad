"""Versioned credential envelopes; no filesystem or HF dependencies."""
import base64
import hashlib
import json
import os
import zlib

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF


def encode(value):
    return base64.b64encode(value).decode("ascii")


def decode(value):
    return base64.b64decode(value, validate=True)


class Vault:
    def __init__(self, key, installation):
        self.root = decode(key)
        if len(self.root) != 32:
            raise ValueError("LP_SPACE_BACKUP_KEY must be base64 of 32 random bytes")
        self.installation = installation
        self.key_id = hashlib.sha256(self.root).hexdigest()[:24]

    def context(self, generation):
        return json.dumps(["launchpad-credentials", 1, self.installation, generation, self.key_id], separators=(",", ":")).encode()

    def cipher(self, generation):
        key = HKDF(algorithm=hashes.SHA256(), length=32, salt=None, info=self.context(generation)).derive(self.root)
        return AESGCM(key)

    def seal(self, generation, payload):
        nonce = os.urandom(12)
        data = zlib.compress(json.dumps(payload, separators=(",", ":")).encode())
        return json.dumps({"format": 1, "key_id": self.key_id, "nonce": encode(nonce),
                           "ciphertext": encode(self.cipher(generation).encrypt(nonce, data, self.context(generation)))}).encode()

    def open(self, generation, envelope):
        value = json.loads(envelope)
        if value.get("format") != 1:
            raise ValueError("Unsupported credential format; existing vault was preserved")
        if value.get("key_id") != self.key_id:
            raise ValueError("Credential key differs; reconnect tools or supply the original key")
        plain = self.cipher(generation).decrypt(decode(value["nonce"]), decode(value["ciphertext"]), self.context(generation))
        return json.loads(zlib.decompress(plain))

    def seal_object(self, payload, seal=None):
        """A self-contained envelope for objects overwritten in place: the seal ID travels with the ciphertext."""
        seal = seal or os.urandom(16).hex()
        return json.dumps(dict(json.loads(self.seal(seal, payload)), seal=seal)).encode()

    def open_object(self, envelope):
        seal = json.loads(envelope).get("seal")
        if not isinstance(seal, str) or not seal:
            raise ValueError("Unsupported credential format; existing vault was preserved")
        return self.open(seal, envelope)


def key_id(envelope):
    return json.loads(envelope).get("key_id")


def seal_id(envelope):
    return json.loads(envelope).get("seal")
