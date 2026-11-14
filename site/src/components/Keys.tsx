/** A shortcut like "Cmd+Shift+D" as keycaps. Separate chords with ", ". */
export default function Keys({ combo }: { combo: string }) {
  return (
    <span className="keys">
      {combo.split(', ').map(chord => <kbd key={chord}>{chord}</kbd>)}
    </span>
  )
}
