#!/usr/bin/env bash
# Cut nex's embedded JetBrains Mono Nerd Font down to the glyphs the site uses (Latin, box
# drawing, blocks, braille, plus any other symbol in src/) as small woff2 files.
# Needs python3 with fonttools and brotli. Run after adding new symbols to the site.
set -euo pipefail
shopt -s globstar
site=$(cd "$(dirname "$0")/.." && pwd)
fonts="$site/../assets/fonts"
extra=$(cat "$site"/src/**/*.ts* "$site"/src/*.ts* 2>/dev/null | python3 -c 'import sys; print("".join(sorted({c for c in sys.stdin.read() if ord(c) > 0x7e})))')
for style in Regular Bold; do
  out="$site/public/fonts/nex-mono-$(echo "$style" | tr A-Z a-z).woff2"
  python3 -m fontTools.subset "$fonts/JetBrainsMonoNLNerdFontMono-$style.ttf" \
    --unicodes='U+0020-007E,U+00A0-00FF,U+2500-259F,U+2800-28FF' --text="$extra" \
    --flavor=woff2 --layout-features='*' --output-file="$out"
done
ls -la "$site/public/fonts"
