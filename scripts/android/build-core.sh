#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MESH="$ROOT/mesh"
OUT="${1:-$MESH/target/android-core}"
mkdir -p "$OUT/kotlin" "$OUT/jniLibs"
cd "$MESH"

export CARGO_INCREMENTAL=0
cargo build --locked -p zeron-mobile --lib --profile mobile
cargo build --locked -p zeron-mobile --bin uniffi-bindgen --features bindgen --profile mobile

if [[ "$(uname)" == "Darwin" ]]; then
  HOST_LIB="$MESH/target/mobile/libzeron_mobile.dylib"
else
  HOST_LIB="$MESH/target/mobile/libzeron_mobile.so"
fi

"$MESH/target/mobile/uniffi-bindgen" generate \
  --library "$HOST_LIB" \
  --language kotlin \
  --config "$ROOT/apps/android/uniffi.toml" \
  --no-format \
  --out-dir "$OUT/kotlin"

python3 - "$OUT/kotlin/uniffi/zeron_core/zeron_core.kt" <<'PY'
import re
import sys
from pathlib import Path

bindings = Path(sys.argv[1])
source = bindings.read_text()
source = source.replace("fun `close`(", "fun closeView(")
source = source.replace("val `message`: kotlin.String", "override val message: kotlin.String")
source = source.replace(
    "private class JavaLangRefCleaner : UniffiCleaner {",
    "@android.annotation.TargetApi(33)\nprivate class JavaLangRefCleaner : UniffiCleaner {",
)
source = source.replace(
    "private class JavaLangRefCleanable(",
    "@android.annotation.TargetApi(33)\nprivate class JavaLangRefCleanable(",
)
source = re.sub(
    r'\n\s*override val message\s+get\(\) = "message=\$\{ `message` \}"',
    "",
    source,
)
bindings.write_text(source)
PY

if ! command -v cargo-ndk >/dev/null; then
  echo "cargo-ndk is required: cargo install cargo-ndk" >&2
  exit 1
fi
if [[ ! -d "${ANDROID_NDK_HOME:-}" ]]; then
  echo "ANDROID_NDK_HOME must point to the installed Android NDK" >&2
  exit 1
fi

ABIS=()
for abi in ${DOVE_ANDROID_ABIS:-arm64-v8a x86_64}; do
  ABIS+=(-t "$abi")
done
CARGO_PROFILE_MOBILE_STRIP=debuginfo CARGO_PROFILE_MOBILE_DEBUG=false \
  cargo ndk "${ABIS[@]}" -o "$OUT/jniLibs" \
  build --locked -p zeron-mobile --lib --profile mobile

echo "Kotlin bindings: $OUT/kotlin"
echo "Android libraries: $OUT/jniLibs"
