import Reveal from '../components/Reveal'
import { desktopOf, usePlatform } from '../platform'
import './agents.css'

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
    body: 'nex-host spawn starts a helper, send asks it something, read shows its screen, done reports back and ls draws the tree. Every message lands in the agent’s input.',
  },
  {
    title: 'Worktrees and skills',
    body: 'nex-host worktree ls and prune keep agents’ git worktrees tidy. nex-host skill install teaches Claude Code and Codex all of this on their own.',
  },
  {
    title: 'Agents outlive the pane',
    body: 'On an ssh host, agents keep running when you close the pane, drop the connection or quit nex. Click a Detached card to pick up where it left off.',
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

type Card = { name: string; task: string; where: string; time: string }

const GROUPS: { status: 'needs' | 'working' | 'done'; label: string; cards: Card[] }[] = [
  { status: 'needs', label: 'Needs you', cards: [{ name: 'export', task: 'Quote every CSV field?', where: 'devbox · ~/todo', time: '12s' }] },
  {
    status: 'working',
    label: 'Working',
    cards: [
      { name: 'search', task: 'Add a search command', where: 'devbox · ~/todo', time: '1m' },
      { name: 'Fix flaky login test', task: 'claude', where: 'Tab 2 · ~/app', time: '4m' },
    ],
  },
  { status: 'done', label: 'Done', cards: [{ name: 'stats', task: 'Opened PR #12', where: 'devbox · ~/todo', time: '2m' }] },
]

const STAGES: { name: string; state: 'done' | 'working' | 'needs' | 'waiting' }[] = [
  { name: 'plan', state: 'done' },
  { name: 'stats', state: 'done' },
  { name: 'search', state: 'working' },
  { name: 'export', state: 'needs' },
  { name: 'merge', state: 'working' },
]

export default function Agents() {
  const platform = desktopOf(usePlatform())
  const mod = platform === 'mac' ? 'Cmd' : 'Ctrl'
  return (
    <section className="section agents" id="agents">
      <Reveal className="agents-head">
        <p className="eyebrow">Built in · Agents</p>
        <h2 className="title">Your agents,{' '}<br />in view.</h2>
        <p className="lede">
          Start Claude Code or Codex in as many panes as you like. Press <kbd>{mod}+Shift+A</kbd> and
          nex lists every one of them, says which one needs you, and takes you to its pane.
          Chain them into pipelines that plan, build, review and merge on their own.
        </p>
      </Reveal>

      <Reveal className="glass agents-card">
        {/* A mock Agents pane: the pipeline's team card, then cards grouped by status. */}
        <div
          className="agents-pane"
          role="img"
          aria-label="The Agents pane, pinned: a pipeline's team card with five stages, then agents grouped as needs you, working and done"
        >
          <div className="agents-pane-head">
            <span className="agents-pin" aria-hidden="true" />Agents
            <span className="agents-count">5 running · 1 needs you</span>
          </div>
          <div className="agents-team">
            <div className="agents-team-top">
              <span className="agents-team-name">demo</span>
              <span className="agents-team-task">pipeline · add stats, search and export</span>
            </div>
            <div className="agents-steps">
              {STAGES.map(s => (
                <span key={s.name} className={`agents-step is-${s.state}`}>{s.name}</span>
              ))}
            </div>
          </div>
          {GROUPS.map(g => (
            <div key={g.label} className={`agents-group is-${g.status}`}>
              <div className="agents-group-label">
                <span className="agents-dot" />{g.label}<span className="agents-n">{g.cards.length}</span>
              </div>
              <div className="agents-cards">
                {g.cards.map(c => (
                  <div key={c.name} className="agents-mini">
                    <div className="agents-mini-top"><b>{c.name}</b><span>{c.time}</span></div>
                    <div className="agents-mini-task">{c.task}</div>
                    <div className="agents-mini-where">{c.where}</div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        <div className="agents-pipe">
          <h3>Pipelines, in a file</h3>
          <p>
            Each stage is an agent with a task, and its report feeds the stages after it. Stages
            can run side by side in worktrees of their own, retry until a reviewer approves, or
            line up behind one merger.
          </p>
          <pre className="agents-code"><code>{PIPELINE}</code></pre>
          <p className="agents-run"><code>nex-host pipe feature "add --json to ls"</code></p>
          <p className="agents-examples">
            Two examples ship with nex-host: <code>feature</code> builds a feature in a worktree,
            opens a PR, reviews and merges it; <code>pr-team</code> lines three builders up behind
            one merger.
          </p>
        </div>
      </Reveal>

      <div className="agents-points">
        {POINTS.map((p, i) => (
          <Reveal key={p.title} delay={i * 70} className="agents-point">
            <h3>{p.title}</h3>
            <p>{p.body}</p>
          </Reveal>
        ))}
      </div>
    </section>
  )
}
