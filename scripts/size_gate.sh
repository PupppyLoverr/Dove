#!/usr/bin/env bash
# Fails when a shipped artifact (file or unpacked folder) is over the size
# budget in docs/ROADMAP.md §3. Warns, without failing, when the main binary
# is over its own budget.
#
#   scripts/size_gate.sh [--binary path] artifact...
#
# Budgets can be overridden with DOVE_SIZE_LIMIT_MB / DOVE_BINARY_LIMIT_MB.
set -euo pipefail

limit_mb="${DOVE_SIZE_LIMIT_MB:-110}"
binary_limit_mb="${DOVE_BINARY_LIMIT_MB:-75}"
binary=""
if [[ "${1:-}" == "--binary" ]]; then
  binary="$2"
  shift 2
fi

mb() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1048576 }'; }

bytes_of() {
  if [[ -d "$1" ]]; then
    # Sum per-file sizes; skip the "total" lines `wc` adds per batch.
    find "$1" -type f -exec wc -c {} + | awk '$2 != "total" { s += $1 } END { print s + 0 }'
  else
    wc -c < "$1" | tr -d ' '
  fi
}

failed=0
checked=0
for path in "$@"; do
  [[ -e "$path" ]] || continue
  checked=$((checked + 1))
  size=$(bytes_of "$path")
  if (( size > limit_mb * 1048576 )); then
    echo "::error::$path is $(mb "$size") MB, over the ${limit_mb} MB budget"
    failed=1
  else
    echo "$path: $(mb "$size") MB (budget ${limit_mb} MB)"
  fi
done

if (( checked == 0 )); then
  echo "::error::size gate found none of: $*"
  exit 1
fi

if [[ -n "$binary" && -f "$binary" ]]; then
  size=$(bytes_of "$binary")
  if (( size > binary_limit_mb * 1048576 )); then
    echo "::warning::$binary is $(mb "$size") MB, over the ${binary_limit_mb} MB binary budget"
  else
    echo "$binary: $(mb "$size") MB (budget ${binary_limit_mb} MB)"
  fi
fi

exit "$failed"
