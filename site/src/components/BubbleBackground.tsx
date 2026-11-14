import './bubbles.css'

/** The blurred gradient blobs that drift behind the whole page. Purely decorative. */
export default function BubbleBackground() {
  return (
    <div className="bubbles" aria-hidden="true">
      <span className="bubble b1" />
      <span className="bubble b2" />
      <span className="bubble b3" />
      <span className="bubble b4" />
      <span className="bubble b5" />
      <div className="bubbles-fade" />
    </div>
  )
}
