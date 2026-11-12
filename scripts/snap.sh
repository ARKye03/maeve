#!/usr/bin/env bash
# Usage: scripts/snap.sh [compare|history|settings]
# Writes target/snaps/{light,dark}-{es,en}[-<screen>].png using the demo fixture;
# the compare screen also gets -mid (scrolled half way), -end (bottom, debate open)
# and -failed (failed row in paste mode).
set -euo pipefail

cd "$(dirname "$0")/.."

screen="${1:-}"
case "$screen" in
  ""|compare|history|settings) ;;
  *) echo "unknown screen: $screen (compare|history|settings)" >&2; exit 2 ;;
esac

cargo build --quiet
bin="target/debug/maeve"
out="target/snaps"
mkdir -p "$out"

for theme in light dark; do
  for lang in es en; do
    name="$theme-$lang${screen:+-$screen}"
    MAEVE_DEMO=1 MAEVE_THEME="$theme" MAEVE_LANG="$lang" MAEVE_SCREEN="$screen" \
      MAEVE_SNAPSHOT="$out/$name.png" "$bin"
    echo "$out/$name.png"
    if [ -z "$screen" ] || [ "$screen" = compare ]; then
      MAEVE_DEMO=1 MAEVE_THEME="$theme" MAEVE_LANG="$lang" MAEVE_SCROLL=0.5 \
        MAEVE_SNAPSHOT="$out/$name-mid.png" "$bin"
      MAEVE_DEMO=1 MAEVE_THEME="$theme" MAEVE_LANG="$lang" MAEVE_SCROLL=1 MAEVE_DEBATE=1 \
        MAEVE_SNAPSHOT="$out/$name-end.png" "$bin"
      MAEVE_DEMO=1 MAEVE_FAILED=1 MAEVE_THEME="$theme" MAEVE_LANG="$lang" \
        MAEVE_SNAPSHOT="$out/$name-failed.png" "$bin"
      echo "$out/$name-mid.png $out/$name-end.png $out/$name-failed.png"
    fi
  done
done
