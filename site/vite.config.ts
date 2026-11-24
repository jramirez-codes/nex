import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { demos } from './scripts/demos.mjs'

// Served from https://jramirez-codes.github.io/nex/ by GitHub Pages.
export default defineConfig({
  base: '/nex/',
  plugins: [react()],
  // The recorded clips, checked at build time (see components/DemoVideo.tsx).
  define: { __DEMOS__: JSON.stringify(demos()) },
})
