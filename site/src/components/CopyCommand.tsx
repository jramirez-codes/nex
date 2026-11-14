import { useState } from 'react'

/** A shell snippet with a copy button. */
export default function CopyCommand({ lines }: { lines: string[] }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(lines.join('\n'))
      setCopied(true)
      setTimeout(() => setCopied(false), 1600)
    } catch {
      // Clipboard can be unavailable (insecure context); the text is still selectable.
    }
  }
  return (
    <div className="cmd">
      <pre>{lines.map(l => <div key={l}><span className="cmd-prompt">$ </span>{l}</div>)}</pre>
      <button type="button" onClick={copy} aria-label="Copy commands">{copied ? 'Copied' : 'Copy'}</button>
    </div>
  )
}
