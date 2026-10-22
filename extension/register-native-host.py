#!/usr/bin/env python3
"""Register this checkout's pinned Launchpad extension with Google Chrome. No daemon restart."""
import argparse
import json
import os
from pathlib import Path
import shlex
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", required=True, type=Path, help="Absolute path to the built launchpad CLI")
parser.add_argument("--environment", choices=["prod", "dev"], default="prod")
parser.add_argument("--port", type=int, help="Override the daemon port (isolated checks)")
args = parser.parse_args()
binary = args.binary.expanduser().resolve(strict=True)
manifest = json.loads((Path(__file__).parent / "public/manifest.json").read_text())
import base64
import hashlib
digest = hashlib.sha256(base64.b64decode(manifest["key"])).hexdigest()[:32]
extension_id = "".join(chr(ord("a") + int(c, 16)) for c in digest)
home = Path.home()
if sys.platform == "darwin":
    target = home / "Library/Application Support/Google/Chrome/NativeMessagingHosts"
elif sys.platform == "win32":
    target = Path(os.environ["LOCALAPPDATA"]) / "Launchpad/NativeMessagingHosts"
else:
    target = home / ".config/google-chrome/NativeMessagingHosts"
target.mkdir(parents=True, exist_ok=True)
launcher = target / ("launchpad-native-host.cmd" if sys.platform == "win32" else "launchpad-native-host")
if sys.platform == "win32":
    script = f'@echo off\nset "LAUNCHPAD_ENV={args.environment}"\n'
    if args.port:
        script += f'set "LAUNCHPAD_PORT={args.port}"\n'
    script += f'"{binary}" native-host %*\n'
else:
    script = f"#!/bin/sh\nexport LAUNCHPAD_ENV={shlex.quote(args.environment)}\n"
    if args.port:
        script += f"export LAUNCHPAD_PORT={args.port}\n"
    script += f'exec {shlex.quote(str(binary))} native-host "$@"\n'
launcher.write_text(script)
launcher.chmod(0o700)
host = target / "dev.launchpad.browser.json"
host.write_text(json.dumps({"name": "dev.launchpad.browser", "description": "Launchpad local browser pairing", "path": str(launcher), "type": "stdio", "allowed_origins": [f"chrome-extension://{extension_id}/"]}, indent=2) + "\n")
if sys.platform == "win32":
    import winreg
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, r"Software\Google\Chrome\NativeMessagingHosts\dev.launchpad.browser") as key:
        winreg.SetValueEx(key, "", 0, winreg.REG_SZ, str(host))
print(f"Registered {host} for extension {extension_id}")
