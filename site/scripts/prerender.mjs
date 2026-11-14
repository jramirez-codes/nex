// Renders the app to static HTML and writes it into dist/index.html, so search engines and
// link previews see the full page without running JavaScript. main.tsx then hydrates it.
// Run after `vite build` and `vite build --ssr` (see package.json).
import { readFile, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const ssrDir = `${root}dist-ssr`
const { render, faqJsonLd } = await import(pathToFileURL(`${ssrDir}/entry-server.js`).href)

const htmlPath = `${root}dist/index.html`
let html = await readFile(htmlPath, 'utf8')
const fill = (marker, value) => {
  if (!html.includes(marker)) throw new Error(`prerender: ${marker} missing from index.html`)
  html = html.replace(marker, () => value)
}
const jsonLd = JSON.stringify(faqJsonLd()).replace(/</g, '\\u003c')
fill('<!--app-html-->', render())
fill('<!--faq-json-ld-->', `<script type="application/ld+json">${jsonLd}</script>`)
await writeFile(htmlPath, html)
await rm(ssrDir, { recursive: true, force: true })
console.log('prerendered dist/index.html')
