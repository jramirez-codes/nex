import { StrictMode } from 'react'
import { hydrateRoot } from 'react-dom/client'
import App from './App'
import './styles/global.css'

// index.html sets this before paint; repeated here in case that inline script is blocked.
document.documentElement.classList.add('js')

// The page is prerendered at build time (scripts/prerender.mjs); this attaches React to it.
hydrateRoot(
  document.getElementById('root')!,
  <StrictMode>
    <App />
  </StrictMode>,
)
