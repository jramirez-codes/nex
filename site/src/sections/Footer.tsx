import { REPO } from '../links'
import { download, platformIcon, usePlatform } from '../platform'
import Icon from '../components/Icon'
import AnimatedLogo from '../components/AnimatedLogo'
import './footer.css'

export default function Footer() {
  const platform = usePlatform()
  return (
    <footer className="footer">
      <div className="footer-cta">
        <AnimatedLogo size={72} onView />
        <p className="footer-title">Try nex today.</p>
        <p>Free and open source, for macOS, Linux and Android, with an iPhone beta. Coming next: custom keybindings, shell integration, themes, a command palette, color emoji and a signed, notarized Mac app.</p>
        <div className="footer-actions">
          <a className="btn btn-primary" href={download(platform).href}>
            <Icon name="download" size={18} />{download(platform).label}<Icon name={platformIcon(platform)} size={18} />
          </a>
          <a className="btn btn-link" href={REPO}><Icon name="github" size={18} />Star on GitHub</a>
        </div>
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
