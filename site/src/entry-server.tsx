import { StrictMode } from 'react'
import { renderToString } from 'react-dom/server'
import App from './App'
import { FAQ } from './data/faq'

/** Used by scripts/prerender.mjs at build time, so the shipped HTML has the whole page in it. */
export function render() {
  return renderToString(<StrictMode><App /></StrictMode>)
}

/** FAQPage structured data, built from the same list the FAQ section renders. */
export function faqJsonLd() {
  return {
    '@context': 'https://schema.org',
    '@type': 'FAQPage',
    mainEntity: FAQ.map(({ q, a }) => ({
      '@type': 'Question',
      name: q,
      acceptedAnswer: { '@type': 'Answer', text: a },
    })),
  }
}
