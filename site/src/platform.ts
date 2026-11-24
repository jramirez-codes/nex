import { useSyncExternalStore } from 'react'
import type { IconName } from './components/Icon'
import { RELEASES, REPO } from './links'

/**
 * Which OS the page is written for: shortcuts, install steps and the download button.
 * Guessed from the browser, overridable with any PlatformToggle, and remembered per visitor.
 * One store, so every toggle on the page moves together.
 */
export type Platform = 'mac' | 'linux' | 'android' | 'ios'
/** The desktop OSes, the ones with keyboard shortcuts. */
export type Desktop = 'mac' | 'linux'

/** Every platform, for the Install section's OS switch. */
export const ALL_PLATFORMS: { id: Platform; label: string; icon: IconName }[] = [
  { id: 'mac', label: 'macOS', icon: 'apple' },
  { id: 'linux', label: 'Linux', icon: 'linux' },
  { id: 'android', label: 'Android', icon: 'android' },
  { id: 'ios', label: 'iOS', icon: 'apple' },
]

/** The desktop platforms, for the shortcut switches (PlatformToggle). */
export const PLATFORMS = ALL_PLATFORMS.filter(p => p.id === 'mac' || p.id === 'linux') as { id: Desktop; label: string; icon: IconName }[]

export const platformName = (p: Platform) => ALL_PLATFORMS.find(x => x.id === p)!.label
export const platformIcon = (p: Platform) => ALL_PLATFORMS.find(x => x.id === p)!.icon

/** Whose shortcuts to show: a phone visitor is shown the desktop they most likely use. */
export const desktopOf = (p: Platform): Desktop => (p === 'mac' || p === 'ios' ? 'mac' : 'linux')

/** The iPhone app has no download yet: it is built from source. */
export const IOS_SRC = `${REPO}/tree/main/nex-ios`

/** The main download button for a platform. */
export const download = (p: Platform) =>
  p === 'ios'
    ? { href: IOS_SRC, label: 'Build the iPhone beta', short: 'iPhone beta' }
    : { href: RELEASES, label: `Download for ${platformName(p)}`, short: platformName(p) }

const KEY = 'nex-platform'
const listeners = new Set<() => void>()
let current: Platform | null = null

const isPlatform = (v: unknown): v is Platform => ALL_PLATFORMS.some(p => p.id === v)

function detect(): Platform {
  try {
    const saved = localStorage.getItem(KEY)
    if (isPlatform(saved)) return saved
  } catch {
    // Storage can be blocked; fall through to the guess.
  }
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } }
  const ua = navigator.userAgent
  const hint = `${nav.userAgentData?.platform ?? ''} ${navigator.platform} ${ua}`
  if (/android/i.test(hint)) return 'android'
  if (/iphone|ipad|ipod/i.test(hint)) return 'ios'
  // iPadOS asks for desktop sites with a Mac user agent; a touch screen gives it away.
  if (/mac/i.test(hint)) return navigator.maxTouchPoints > 1 ? 'ios' : 'mac'
  return 'linux'
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
