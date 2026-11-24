import DemoVideo from '../components/DemoVideo'
import Points from '../components/Points'
import Reveal from '../components/Reveal'
import { desktopOf, usePlatform } from '../platform'

/** Copy follows the README's "Agents" section and nex-host/README.md's "Pipelines". */
const POINTS = [
  {
    title: 'Every agent, one pane',
    body: 'Claude Code and Codex in any tab, local or on a server, as cards sorted most urgent first: needs you, done, working, idle.',
  },
  {
    title: 'Pin it to every tab',
    body: 'Pin the Agents pane and it follows you from tab to tab, so you can keep working while the cards keep updating.',
  },
  {
    title: 'Teams and pipelines',
    body: 'An agent that starts helpers, or a pipeline, shows up as one team card with its stages. Open it to see who is on what.',
  },
  {
    title: 'Agents that talk to each other',
    body: 'nex-host spawn starts a helper, send asks it something, read shows its screen, done reports back and ls draws the tree.',
  },
  {
    title: 'Worktrees and skills',
    body: 'nex-host worktree ls and prune keep agents’ git worktrees tidy. nex-host skill install teaches Claude Code and Codex all of this.',
  },
  {
    title: 'Agents outlive the pane',
    body: 'On an ssh host, agents keep running when you close the pane, drop the connection or quit nex. Click a Detached card to pick up again.',
  },
]

const PIPELINE = `# .nex/pipelines/feature.toml
[[stage]]
name = "plan"
task = "Plan this: {task}. Write PLAN.md."

[[stage]]
name = "build"
worktree = true
task = "Implement PLAN.md. In short: {plan}"

[[stage]]
name = "review"
kind = "codex"
task = "Review the diff. done --verdict approved|changes"
retry = { stage = "build", unless = "approved", max = 3 }`

export default function Agents() {
  const platform = desktopOf(usePlatform())
  const mod = platform === 'mac' ? 'Cmd' : 'Ctrl'
  return (
    <section className="section" id="agents">
      <Reveal className="section-head">
        <p className="eyebrow">Agents</p>
        <h2 className="title">Your agents,{' '}<br />in view.</h2>
        <p className="lede">
          Start Claude Code or Codex in as many panes as you like. Press <kbd>{mod}+Shift+A</kbd> and
          nex lists every one of them, says which one needs you, and takes you to its pane.
        </p>
      </Reveal>

      <Reveal>
        <DemoVideo
          name="agents"
          label="A pipeline of agents starts over ssh; Ctrl+Shift+A opens the Agents pane, its team card opens to show every stage, and Enter jumps into one of the helpers"
          fallback={{
            name: 'promo',
            label: 'A pipeline of agents starts over ssh and the Agents pane is pinned beside it; from it the run’s team opens and one of its agents is jumped into',
            size: [1920, 1080],
          }}
          width={1200}
          height={750}
        />
      </Reveal>

      <Points items={POINTS} />

      <Reveal className="aside">
        <div>
          <h3>Pipelines, in a file.</h3>
          <p>
            Each stage is an agent with a task, and its report feeds the stages after it. Stages
            can run side by side in worktrees of their own, retry until a reviewer approves, or
            line up behind one merger. Run it with <code>nex-host pipe feature "add --json to ls"</code>.
          </p>
          <p>
            Two examples ship with nex-host: <code>feature</code> builds a feature in a worktree,
            opens a PR, reviews and merges it; <code>pr-team</code> lines three builders up behind
            one merger.
          </p>
        </div>
        <pre className="code"><code>{PIPELINE}</code></pre>
      </Reveal>
    </section>
  )
}
