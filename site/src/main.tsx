import { StrictMode } from 'react'
import { hydrateRoot } from 'react-dom/client'
import App from './App'
import './styles/global.css'

// The page is prerendered at build time (scripts/prerender.mjs); this attaches React to it.
hydrateRoot(
  document.getElementById('root')!,
  <StrictMode>
    <App />
  </StrictMode>,
)
