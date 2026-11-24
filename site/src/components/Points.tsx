import Reveal from './Reveal'

/** The short plain list under a section's recording: a title and a line or two each. */
export default function Points({ items }: { items: { title: string; body: string }[] }) {
  return (
    <ul className="points">
      {items.map((p, i) => (
        <Reveal as="li" key={p.title} delay={(i % 3) * 70}>
          <h3>{p.title}</h3>
          <p>{p.body}</p>
        </Reveal>
      ))}
    </ul>
  )
}
