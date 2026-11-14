import { useSyncExternalStore } from 'react'

/**
 * Which OS the page is written for: shortcuts, install steps and the download button.
 * Guessed from the browser, overridable with any PlatformToggle, and remembered per visitor.
 * One store, so every toggle on the page moves together.
 */
export type Platform = 'mac' | 'linux'

export const PLATFORMS: { id: Platform; label: string }[] = [
  { id: 'mac', label: 'macOS' },
  { id: 'linux', label: 'Linux' },
]

export const platformName = (p: Platform) => (p === 'mac' ? 'macOS' : 'Linux')

const KEY = 'nex-platform'
const listeners = new Set<() => void>()
let current: Platform | null = null

function detect(): Platform {
  try {
    const saved = localStorage.getItem(KEY)
    if (saved === 'mac' || saved === 'linux') return saved
  } catch {
    // Storage can be blocked; fall through to the guess.
  }
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } }
  const hint = nav.userAgentData?.platform || navigator.platform || navigator.userAgent
  // iPhone and iPad visitors are most likely Mac users browsing on the go.
  return /mac|iphone|ipad|ipod/i.test(hint) ? 'mac' : 'linux'
}

function subscribe(fn: () => void) {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

const getSnapshot = () => (current ??= detect())
// The prerendered HTML is written for macOS, the newest platform; the client corrects it on load.
const getServerSnapshot = (): Platform => 'mac'

export function setPlatform(p: Platform) {
  current = p
  try {
    localStorage.setItem(KEY, p)
  } catch {
    // Not remembered, but still applied for this visit.
  }
  listeners.forEach(fn => fn())
}

export const usePlatform = () => useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot)
