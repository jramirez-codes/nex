import { REPO, asset } from '../links'
import './footer.css'

export default function Footer() {
  return (
    <footer className="footer">
      <div className="footer-cta">
        <img src={asset('nex.svg')} alt="" width={72} height={72} />
        <h2>Try nex today.</h2>
        <p>Free and open source. Now on macOS too. Coming next: a quick drop-down terminal, a command palette and shell integration.</p>
        <a className="btn btn-primary" href={REPO}>Star on GitHub</a>
      </div>
      <div className="footer-meta">
        <p>
          MIT licensed. Built on <a href="https://github.com/alacritty/alacritty">alacritty_terminal</a> and{' '}
          <a href="https://github.com/emilk/egui">egui</a>, with defaults inspired by <a href="https://ghostty.org">Ghostty</a>.
          JetBrains Mono and Nerd Fonts under the SIL Open Font License.
        </p>
        <p><a href={REPO}>GitHub</a> · <a href={`${REPO}/releases`}>Releases</a> · <a href={`${REPO}/issues`}>Report an issue</a></p>
      </div>
    </footer>
  )
}
