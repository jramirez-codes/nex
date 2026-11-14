#!/usr/bin/env bash
# Render assets/nex.svg into packaging/macos/nex.icns, keeping the transparent corners.
#
#   packaging/macos/make-icns.sh
#
# Needs macOS 14+ (AppKit renders the SVG) and Xcode's command line tools (swift, iconutil).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SVG="$HERE/../../assets/nex.svg"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SET="$WORK/nex.iconset"
mkdir -p "$SET"

cat > "$WORK/render.swift" <<'EOF'
import AppKit

// render.swift SVG OUT_DIR: writes every size an .iconset needs, on a clear background.
let args = CommandLine.arguments
guard let svg = NSImage(contentsOfFile: args[1]) else {
    FileHandle.standardError.write("cannot read \(args[1])\n".data(using: .utf8)!)
    exit(1)
}
for (points, scale) in [16, 32, 128, 256, 512].flatMap({ p in [(p, 1), (p, 2)] }) {
    let px = points * scale
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSColor.clear.set()
    NSRect(x: 0, y: 0, width: px, height: px).fill(using: .copy)
    svg.draw(in: NSRect(x: 0, y: 0, width: px, height: px))
    NSGraphicsContext.restoreGraphicsState()
    let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(args[2])/\(name)"))
}
EOF

swiftc -O -framework AppKit "$WORK/render.swift" -o "$WORK/render"
"$WORK/render" "$SVG" "$SET"
iconutil -c icns "$SET" -o "$HERE/nex.icns"
echo "Wrote $HERE/nex.icns"
