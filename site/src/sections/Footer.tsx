import { REPO } from '../links'
import './footer.css'

export default function Footer() {
  return (
    <footer className="footer">
      <p>
        MIT licensed. Built on <a href="https://github.com/alacritty/alacritty">alacritty_terminal</a> and{' '}
        <a href="https://github.com/emilk/egui">egui</a>, with defaults inspired by <a href="https://ghostty.org">Ghostty</a>.
        JetBrains Mono and Nerd Fonts under the SIL Open Font License.
      </p>
      <p><a href={REPO}>GitHub</a> · <a href={`${REPO}/releases`}>Releases</a> · <a href={`${REPO}/issues`}>Report an issue</a> · <a href="/nex/privacy/">Privacy</a></p>
    </footer>
  )
}
