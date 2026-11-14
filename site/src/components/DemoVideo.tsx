import { useEffect, useRef } from 'react'
import { asset } from '../links'

/**
 * A looping, muted screen recording from public/demo/ (`<name>.mp4` with a `<name>.jpg` poster).
 * MP4 is a fraction of the size of the GIFs it replaces. Unless `eager`, it only downloads and
 * plays while on screen, which keeps the first load light.
 */
export default function DemoVideo({ name, label, width, height, eager = false, className }: {
  name: string
  label: string
  width: number
  height: number
  eager?: boolean
  className?: string
}) {
  const ref = useRef<HTMLVideoElement>(null)

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
  }, [eager])

  return (
    <video
      ref={ref}
      className={className}
      src={asset(`demo/${name}.mp4`)}
      poster={asset(`demo/${name}.jpg`)}
      aria-label={label}
      title={label}
      width={width}
      height={height}
      autoPlay={eager}
      preload={eager ? 'auto' : 'none'}
      muted
      loop
      playsInline
    />
  )
}
