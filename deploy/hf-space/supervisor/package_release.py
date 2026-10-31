"""Build a minimal public template from a published immutable image, not app source."""
import argparse
import json
import zipfile
from pathlib import Path
from supervisor.updates import dockerfile

parser=argparse.ArgumentParser()
parser.add_argument('--image',required=True)
parser.add_argument('--revision',required=True)
parser.add_argument('--notes',default='https://github.com/Wauplin/launchpad/commits/main')
parser.add_argument('--output',required=True,type=Path)
args=parser.parse_args()
root=Path(__file__).resolve().parents[1]
release=json.loads((root/'release.json').read_text())
release.update(image=args.image,launchpad=args.revision,notes=args.notes,database='Full pre-update checkpoint required; rollback restores the matching data.')
args.output.mkdir(parents=True,exist_ok=True)
(args.output/'hf-space-release.json').write_text(json.dumps(release,indent=2)+'\n')
with zipfile.ZipFile(args.output/'hf-space-template.zip','w') as archive:
    archive.writestr('Dockerfile',dockerfile(args.image))
    archive.writestr('README.md',(root/'template/README.md').read_text())
print('Release manifest and duplicate template generated')
