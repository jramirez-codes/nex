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
// SEO guards: fail the build rather than ship a truncated title or broken structured data.
const title = html.match(/<title>([^<]*)<\/title>/)?.[1].replace(/&amp;/g, '&') ?? ''
const description = html.match(/<meta name="description" content="([^"]*)"/)?.[1] ?? ''
if (!title || title.length > 60) throw new Error(`prerender: <title> is ${title.length} chars (max 60)`)
if (!description || description.length > 160) throw new Error(`prerender: meta description is ${description.length} chars (max 160)`)
for (const [, block] of html.matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g)) {
  try { JSON.parse(block) } catch (e) { throw new Error(`prerender: invalid JSON-LD: ${e.message}`) }
}
await writeFile(htmlPath, html)

// Stamp the build date into the sitemap so crawlers see when the page last changed.
const sitemapPath = `${root}dist/sitemap.xml`
const lastmod = new Date().toISOString().slice(0, 10)
const sitemap = await readFile(sitemapPath, 'utf8')
if (!sitemap.includes('</loc>')) throw new Error('prerender: no <loc> in sitemap.xml')
await writeFile(sitemapPath, sitemap.replace(/<\/loc>(?!\s*<lastmod>)/g, `</loc>\n    <lastmod>${lastmod}</lastmod>`))
await rm(ssrDir, { recursive: true, force: true })
console.log(`prerendered dist/index.html, sitemap lastmod ${lastmod}`)
