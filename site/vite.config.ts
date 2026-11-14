import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Served from https://jramirez-codes.github.io/nex/ by GitHub Pages.
export default defineConfig({
  base: '/nex/',
  plugins: [react()],
})
