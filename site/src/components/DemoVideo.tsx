import { useEffect, useRef } from 'react'
import { asset } from '../links'

export interface Clip {
  name: string
  label: string
  /** The clip's own size, when a fallback isn't the same shape as the clip it stands in for. */
  size?: [number, number]
}

/** Whether public/demo/ has `<name>.mp4` and its `<name>.jpg` poster (checked at build time). */
export const hasDemo = (name: string) => __DEMOS__.includes(name)

/** The first of `clips` that was recorded, or null if none was. */
export const firstDemo = (...clips: (Clip | undefined)[]): Clip | null =>
  clips.find((c): c is Clip => !!c && hasDemo(c.name)) ?? null

/**
 * A looping, muted screen recording from public/demo/ (`<name>.mp4` with a `<name>.jpg` poster),
 * in a plain rounded frame. MP4 is a fraction of the size of the GIFs it replaces. Unless
 * `eager`, it only downloads and plays while on screen, which keeps the first load light.
 * If `name` hasn't been recorded yet, `fallback` plays instead, and if neither has, nothing
 * renders: a missing clip never shows as a broken video.
 */
export default function DemoVideo({ name, label, fallback, width, height, eager = false, className = '' }: Clip & {
  fallback?: Clip
  width: number
  height: number
  eager?: boolean
  className?: string
}) {
  const ref = useRef<HTMLVideoElement>(null)
  const clip = firstDemo({ name, label }, fallback)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      el.pause()
      return
    }
    if (eager) return
    const io = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) el.play().catch(() => {})
      else el.pause()
    }, { threshold: 0.25 })
    io.observe(el)
    return () => io.disconnect()
  }, [eager, clip?.name])

  if (!clip) return null
  return (
    <div className={`frame ${className}`}>
      <video
        ref={ref}
        src={asset(`demo/${clip.name}.mp4`)}
        poster={asset(`demo/${clip.name}.jpg`)}
        aria-label={clip.label}
        title={clip.label}
        width={clip.size?.[0] ?? width}
        height={clip.size?.[1] ?? height}
        autoPlay={eager}
        preload={eager ? 'auto' : 'none'}
        muted
        loop
        playsInline
      />
    </div>
  )
}
