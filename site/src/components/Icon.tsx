/**
 * Small monochrome icons, drawn inline in `currentColor` so they take the text color around
 * them. No icon font or external request. Decorative by default (aria-hidden); a control
 * that is only an icon gives itself an aria-label.
 */
export type IconName = 'github' | 'download' | 'apple' | 'linux' | 'android' | 'terminal' | 'copy'

const PATHS: Record<IconName, JSX.Element> = {
  github: (
    <path
      fill="currentColor"
      d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.25.73-1.54-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.42-2.69 5.39-5.26 5.68.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5Z"
    />
  ),
  download: (
    <g fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 3v12" />
      <path d="m7 10 5 5 5-5" />
      <path d="M5 20h14" />
    </g>
  ),
  apple: (
    <path
      fill="currentColor"
      d="M16.37 12.6c-.02-2.3 1.88-3.4 1.96-3.46-1.07-1.56-2.73-1.78-3.32-1.8-1.41-.14-2.76.83-3.48.83-.72 0-1.82-.81-3-.79-1.54.02-2.96.9-3.76 2.28-1.6 2.78-.41 6.9 1.15 9.16.76 1.1 1.67 2.34 2.86 2.3 1.15-.05 1.58-.74 2.97-.74 1.38 0 1.77.74 2.98.72 1.23-.02 2.01-1.12 2.77-2.23.87-1.28 1.23-2.52 1.25-2.58-.03-.01-2.4-.92-2.42-3.65ZM14.1 5.86c.63-.77 1.06-1.83.94-2.89-.91.04-2.02.61-2.67 1.37-.58.68-1.1 1.77-.96 2.81 1.02.08 2.06-.52 2.69-1.29Z"
    />
  ),
  linux: (
    <g fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2.5c-2.2 0-3.4 1.8-3.4 4.1 0 1.6-.4 2.6-1.4 4C6 12.2 5 14 5 15.8c0 .7.1 1.3.4 1.9" />
      <path d="M12 2.5c2.2 0 3.4 1.8 3.4 4.1 0 1.6.4 2.6 1.4 4 1.2 1.6 2.2 3.4 2.2 5.2 0 .7-.1 1.3-.4 1.9" />
      <path d="M10.2 7.2h.01M13.8 7.2h.01" strokeWidth="2.2" />
      <path d="M10.6 9.2 12 10.2l1.4-1" />
      <path d="M5.4 17.7c-1.3.4-1.7 1.3-1.2 2 .6.8 2.2.9 3.6 1.4.9.3 1.6.1 1.9-.6.4-.9-.3-2.2-1.2-3-.9-.7-2-.8-3.1.2Z" />
      <path d="M18.6 17.7c1.3.4 1.7 1.3 1.2 2-.6.8-2.2.9-3.6 1.4-.9.3-1.6.1-1.9-.6-.4-.9.3-2.2 1.2-3 .9-.7 2-.8 3.1.2Z" />
      <path d="M9.6 20.6c1.5.5 3.3.5 4.8 0" />
    </g>
  ),
  android: (
    <path
      fill="currentColor"
      d="M17.6 9.48 19.44 6.3a.38.38 0 0 0-.66-.38l-1.87 3.23a11.43 11.43 0 0 0-9.82 0L5.22 5.92a.38.38 0 1 0-.66.38L6.4 9.48A10.78 10.78 0 0 0 1 18h22a10.78 10.78 0 0 0-5.4-8.52ZM7 15.25A1.25 1.25 0 1 1 8.25 14 1.25 1.25 0 0 1 7 15.25Zm10 0A1.25 1.25 0 1 1 18.25 14 1.25 1.25 0 0 1 17 15.25Z"
    />
  ),
  terminal: (
    <g fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2.5" y="4" width="19" height="16" rx="3" />
      <path d="m7 9.5 3 2.5-3 2.5" />
      <path d="M12.5 15h4.5" />
    </g>
  ),
  copy: (
    <g fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="8.5" y="8.5" width="12" height="12" rx="2.5" />
      <path d="M15.5 8.5V6a2.5 2.5 0 0 0-2.5-2.5H6A2.5 2.5 0 0 0 3.5 6v7A2.5 2.5 0 0 0 6 15.5h2.5" />
    </g>
  ),
}

export default function Icon({ name, size = 18, className = '' }: { name: IconName; size?: number; className?: string }) {
  return (
    <svg
      className={`icon ${className}`}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
    >
      {PATHS[name]}
    </svg>
  )
}
