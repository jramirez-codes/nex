import Chapter from '../components/Chapter'
import { desktopOf, usePlatform } from '../platform'

/** Copy follows the README's "Agents" section and nex-host/README.md's "Pipelines". */
const POINTS = [
  {
    title: 'Pin it to every tab',
    body: 'Pin the Agents pane and it follows you from tab to tab, its cards updating while you work.',
  },
  {
    title: 'Teams and pipelines',
    body: 'A pipeline in .nex/pipelines/ runs stages of agents, in worktrees of their own. The run shows up as one team card.',
  },
  {
    title: 'Agents outlive the pane',
    body: 'On an ssh host, agents keep running when you close the pane, drop the connection or quit nex.',
  },
]

export default function Agents() {
  const mod = desktopOf(usePlatform()) === 'mac' ? 'Cmd' : 'Ctrl'
  return (
    <Chapter
      id="agents"
      eyebrow="Agents"
      accent="violet"
      title={['Your agents.', 'All in view.']}
      lede={<>Claude Code and Codex, in any tab or on any server. Press <kbd>{mod}+Shift+A</kbd> to see which one needs you, and jump straight to it.</>}
      clip={{
        name: 'agents',
        label: 'A pipeline of agents starts over ssh; Ctrl+Shift+A opens the Agents pane, its team card opens to show every stage, and Enter jumps into one of the helpers',
      }}
      fallback={{
        name: 'promo',
        label: 'A pipeline of agents starts over ssh and the Agents pane is pinned beside it; from it the run’s team opens and one of its agents is jumped into',
        size: [1920, 1080],
      }}
      points={POINTS}
    />
  )
}
