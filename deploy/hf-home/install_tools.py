"""Image-build only. release.json is the single version source for native CLIs."""
import hashlib
import io
import json
import os
import re
import subprocess
import tarfile
import zipfile
from pathlib import Path

pins=json.loads(Path('/opt/home/release.json').read_text())
versions,digests=pins['tools'],pins['checksums']
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


def install(path, binary):
    Path(path).write_bytes(binary)
    os.chmod(path, 0o755)


def sha256_of(data, expected, what):
    if hashlib.sha256(data).hexdigest() != expected: raise ValueError(f'{what} checksum mismatch')


def from_tar(archive, member):
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as tar:
        return tar.extractfile(member).read()


# Programs Home runs itself: Monty's worker and Ruff for oracles (launchpad_automation::oracle_eval), slackline for
# agents' `launchpad slack` calls (launchpad_core::slack), x86_64 like the rest of the image.
# Monty: the `monty` executable of the pydantic-monty-runtime wheel (the monty-runtime crate's binary), checked against PyPI's digest.
version = versions['monty']
release = json.loads(download(f'https://pypi.org/pypi/pydantic-monty-runtime/{version}/json'))
wheel = next(f for f in release['urls'] if f['filename'] == f'pydantic_monty_runtime-{version}-py3-none-manylinux_2_28_x86_64.whl')
data = download(wheel['url'])
sha256_of(data, wheel['digests']['sha256'], 'Monty')
with zipfile.ZipFile(io.BytesIO(data)) as whl:
    install('/usr/local/bin/monty', whl.read(f'pydantic_monty_runtime-{version}.data/scripts/monty'))

# Ruff: the GitHub release archive and its published checksum.
version = versions['ruff']
url = f'https://github.com/astral-sh/ruff/releases/download/{version}/ruff-x86_64-unknown-linux-gnu.tar.gz'
archive = download(url)
sha256_of(archive, download(url + '.sha256').decode().split()[0], 'Ruff')
install('/usr/local/bin/ruff', from_tar(archive, 'ruff-x86_64-unknown-linux-gnu/ruff'))

# slackline: the GitHub release archive (no checksum file), checked against the digest bump_tools.py pinned in release.json
# from GitHub's asset record. The build never calls api.github.com: the Space builder's shared address hits its rate limit.
version = versions['slackline']
archive = download(f'https://github.com/co42/slackline/releases/download/v{version}/slackline-v{version}-x86_64-unknown-linux-gnu.tar.gz')
sha256_of(archive, digests['slackline'], 'slackline')
install('/usr/local/bin/slackline', from_tar(archive, 'slackline'))

# Funes' models (recall, launchpad_recall::funes): the embedder and the reranker, each file at the revision pinned in
# release.json and checked against its digest, laid out as a Hugging Face cache (`hub/models--<owner>--<name>/snapshots/
# <revision>/`). Home runs funes offline with HF_HOME=/opt/funes/hf (LAUNCHPAD_FUNES_HF_HOME), so the weights never
# change under a memory and are never downloaded at run time. The funes program itself is built in the Dockerfile.
for repo, model in pins['funes']['models'].items():
    revision = model['revision']
    if not re.fullmatch(r'[0-9a-f]{40}', revision): raise ValueError('Model revisions must be full commit IDs')
    folder = Path('/opt/funes/hf/hub') / ('models--' + repo.replace('/', '--')) / 'snapshots' / revision
    folder.mkdir(parents=True, exist_ok=True)
    for name, expected in model['files'].items():
        target = folder / name
        subprocess.run(['curl', '-4', '--fail', '--silent', '--show-error', '--location', '--retry', '3', '--connect-timeout', '20', '--max-time', '1800',
                        '--output', str(target), f'https://huggingface.co/{repo}/resolve/{revision}/{name}'], check=True)
        digest = hashlib.sha256()
        with target.open('rb') as f:
            for chunk in iter(lambda: f.read(1 << 20), b''):
                digest.update(chunk)
        if digest.hexdigest() != expected: raise ValueError(f'{repo}/{name} checksum mismatch')
