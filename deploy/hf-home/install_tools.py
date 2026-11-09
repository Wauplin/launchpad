"""Image-build only. release.json is the single version source for native CLIs."""
import hashlib
import io
import json
import os
import re
import subprocess
import tarfile
from pathlib import Path

versions=json.loads(Path('/opt/home/release.json').read_text())['tools']
if any(not re.fullmatch(r'\d+\.\d+\.\d+',str(v)) for v in versions.values()): raise ValueError('Tool versions must be exact semver releases')
subprocess.run(['npm','install','-g',f"@openai/codex@{versions['codex']}",f"@anthropic-ai/claude-code@{versions['claude']}",f"opencode-ai@{versions['opencode']}"],check=True)
version=versions['gh']
name=f'gh_{version}_linux_amd64.tar.gz'
url=f'https://github.com/cli/cli/releases/download/v{version}/'
def download(url):
    return subprocess.check_output(['curl', '-4', '--fail', '--silent', '--show-error', '--location', '--retry', '2', '--connect-timeout', '20', '--max-time', '120', url])

archive=download(url+name)
checksums=download(url+f'gh_{version}_checksums.txt').decode()
expected=next(line.split()[0] for line in checksums.splitlines() if line.split()[-1]==name)
if hashlib.sha256(archive).hexdigest()!=expected: raise ValueError('GitHub CLI checksum mismatch')
with tarfile.open(fileobj=io.BytesIO(archive),mode='r:gz') as tar:
    binary=tar.extractfile(f'gh_{version}_linux_amd64/bin/gh').read()
Path('/usr/local/bin/gh').write_bytes(binary)
os.chmod('/usr/local/bin/gh',0o755)
