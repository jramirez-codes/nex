import { PLATFORMS, setPlatform, usePlatform } from '../platform'
import Icon from './Icon'

/** macOS / Linux switch. Every instance on the page shares the same state. */
export default function PlatformToggle({ className = '' }: { className?: string }) {
  const platform = usePlatform()
  return (
    <div className={`seg seg-compact ${className}`} role="radiogroup" aria-label="Show for">
      {PLATFORMS.map(p => (
        <button
          key={p.id}
          type="button"
          role="radio"
          aria-checked={p.id === platform}
          className={p.id === platform ? 'on' : ''}
          onClick={() => setPlatform(p.id)}
        >
          <Icon name={p.icon} size={16} />
          {p.label}
        </button>
      ))}
    </div>
  )
}
