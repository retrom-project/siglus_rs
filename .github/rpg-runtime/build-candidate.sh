#!/usr/bin/env bash
set -euo pipefail
siglus_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
python3 "$siglus_root/tools/build_retrom_candidate.py" "${1:?absolute empty output directory is required}"
