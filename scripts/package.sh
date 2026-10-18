#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p dist
python3 - <<'PY'
from pathlib import Path
import zipfile
root=Path.cwd()
destination=root/'dist/teryon-source.zip'
with zipfile.ZipFile(destination,'w',zipfile.ZIP_DEFLATED) as z:
 for p in sorted(root.rglob('*')):
  rel=p.relative_to(root)
  if any(part in {'target','dist','.git','__pycache__'} for part in rel.parts):continue
  if p.is_file():z.write(p,Path('teryon')/rel)
print(destination)
PY
