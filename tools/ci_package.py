"""Run immutable product packaging with this workflow's workspace fixture."""
import argparse
from pathlib import Path
import runpy
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source', type=Path, required=True)
args, rest = parser.parse_known_args()
source = args.source.resolve()/'tools/package.py'
# Keep the workspace fixture from this workflow revision, while all product
# code, manifests, validation and provenance come from the requested tag.
import sdk_workspace
sys.path.insert(0, str(source.parent))
sys.argv = [str(source), *rest]
runpy.run_path(str(source), run_name='__main__')
