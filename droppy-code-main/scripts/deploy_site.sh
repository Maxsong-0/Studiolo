#!/bin/bash
# Rebuilds the site's changelog from ReleaseNotes and deploys the site.
set -euo pipefail

cd "$(dirname "$0")/.."

python3 scripts/build_changelog.py

netlify deploy --prod --site b6307927-958d-4208-822c-2a0677210ce8 --no-build --dir website
