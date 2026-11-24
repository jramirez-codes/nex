// Lists the recordings in public/demo/ that have both a clip and a poster (`<name>.mp4` and
// `<name>.jpg`). vite.config.ts bakes the list into the build as __DEMOS__, so DemoVideo can
// fall back to another clip rather than render a broken video.
import { readdirSync } from 'node:fs'

export function demos() {
  const files = new Set(readdirSync(new URL('../public/demo/', import.meta.url)))
  return [...files]
    .filter(f => f.endsWith('.mp4') && files.has(f.replace(/\.mp4$/, '.jpg')))
    .map(f => f.replace(/\.mp4$/, ''))
    .sort()
}
