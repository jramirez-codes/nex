import { useEffect, useRef, useState, type ReactNode } from 'react'

/** Fades and slides its children in the first time they scroll into view. */
export default function Reveal({ children, delay = 0, className = '', as: Tag = 'div' }: {
  children: ReactNode
  delay?: number
  className?: string
  /** The element to render, e.g. "li" inside a list. */
  as?: 'div' | 'li'
}) {
  const ref = useRef<HTMLDivElement & HTMLLIElement>(null)
  const [shown, setShown] = useState(false)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const io = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) {
        setShown(true)
        io.disconnect()
      }
    }, { threshold: 0.15 })
    io.observe(el)
    return () => io.disconnect()
  }, [])

  return (
    <Tag ref={ref} className={`reveal ${shown ? 'in' : ''} ${className}`} style={{ transitionDelay: `${delay}ms` }}>
      {children}
    </Tag>
  )
}
