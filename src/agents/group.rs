//! Teams: an agent on an ssh host that started others (`nex-host spawn`), or a pipeline and
//! its stages, is shown as one card with its helpers inside it.
//!
//! A team sits in the section of its most urgent member, so a helper that needs you brings
//! the whole team up to Needs You. Inside, helpers keep the order they started in, and a
//! helper changing state never moves the card around: only the team's own status does.
//! Everything else (local panes' agents, agents nobody started) is a team of one: a card as
//! before.

use std::collections::{HashMap, HashSet};

use crate::layout::PaneId;

use super::model::{Agent, AgentKind};

/// One helper in a team, and how far down the tree it is (1: started by the team's lead).
#[derive(Clone, Debug)]
pub struct Member {
    pub agent: Agent,
    pub depth: usize,
}

/// A card: an agent with the helpers it started, if any.
#[derive(Clone, Debug)]
pub struct Group {
    /// The lead (or pipeline) with `status` set to the team's: its most urgent member's.
    pub lead: Agent,
    /// Its helpers, depth first, each level in the order they started.
    pub members: Vec<Member>,
}

impl Group {
    pub fn key(&self) -> PaneId {
        self.lead.pane
    }

    pub fn is_team(&self) -> bool {
        !self.members.is_empty()
    }

    /// A pipeline's stages run in order: they're drawn as steps.
    pub fn is_pipeline(&self) -> bool {
        self.lead.kind == AgentKind::Pipeline
    }

    /// The helpers the lead started itself (what the collapsed card's capsules show).
    pub fn direct(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().filter(|m| m.depth == 1)
    }

    /// Member `i`'s own helpers: the members right after it, deeper down.
    pub fn under(&self, i: usize) -> impl Iterator<Item = &Member> {
        let depth = self.members[i].depth;
        self.members[i + 1..].iter().take_while(move |m| m.depth > depth).filter(move |m| m.depth == depth + 1)
    }

    /// The closed card's capsule row: the lead's helpers, with a pipeline among them opened
    /// into its stages (its name first).
    pub fn strip(&self) -> Vec<Step<'_>> {
        let mut out = Vec::new();
        for (i, m) in self.members.iter().enumerate().filter(|(_, m)| m.depth == 1) {
            let stages: Vec<&Member> = self.under(i).collect();
            if self.is_pipeline() {
                out.push(Step::Stage(&m.agent));
            } else if m.agent.kind == AgentKind::Pipeline && !stages.is_empty() {
                out.push(Step::Heading(&m.agent.name));
                out.extend(stages.into_iter().map(|s| Step::Stage(&s.agent)));
            } else {
                out.push(Step::Helper(&m.agent));
            }
        }
        out
    }

    /// The size the card's header gives: `2 stages` for a pipeline, else its helpers and
    /// pipelines (a nested pipeline counted once, not as its stages).
    pub fn size(&self) -> String {
        let n = |k: usize, what: &str| format!("{k} {what}{}", if k == 1 { "" } else { "s" });
        if self.is_pipeline() {
            return n(self.members.len(), "stage");
        }
        let (pipes, helpers) = self.direct().fold((0, 0), |(p, h), m| if m.agent.kind == AgentKind::Pipeline { (p + 1, h) } else { (p, h + 1) });
        match (helpers, pipes) {
            (h, 0) => n(h, "helper"),
            (0, p) => n(p, "pipeline"),
            (h, p) => format!("{} · {}", n(h, "helper"), n(p, "pipeline")),
        }
    }
}

/// One thing in a closed team's capsule row.
#[derive(Clone, Copy, Debug)]
pub enum Step<'a> {
    /// A nested pipeline's name, before its stages.
    Heading(&'a str),
    /// A pipeline's stage: drawn as steps, joined to the one before unless it's the same
    /// stage ([`same_step`]).
    Stage(&'a Agent),
    Helper(&'a Agent),
}

impl<'a> Step<'a> {
    pub fn agent(self) -> Option<&'a Agent> {
        match self {
            Step::Heading(_) => None,
            Step::Stage(a) | Step::Helper(a) => Some(a),
        }
    }
}

/// Whether two helpers side by side in a row are one step (the same stage, like `build`
/// and `build-2`) rather than one after the other.
pub fn same_step(a: &Agent, b: &Agent) -> bool {
    let role = |a: &Agent| a.team.role.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| a.name.clone());
    role(a) == role(b)
}

/// `agents` (as the tracker sorted them) as cards: each agent under the one that started
/// it, when that one is listed too, and teams sorted by their most urgent member.
pub fn groups(agents: &[Agent]) -> Vec<Group> {
    let keys: HashSet<PaneId> = agents.iter().map(|a| a.pane).collect();
    let parent = |a: &Agent| a.team.parent.filter(|p| *p != a.pane && keys.contains(p));
    let mut children: HashMap<PaneId, Vec<&Agent>> = HashMap::new();
    for a in agents {
        if let Some(p) = parent(a) {
            children.entry(p).or_default().push(a);
        }
    }
    for kids in children.values_mut() {
        kids.sort_by_key(|a| (a.team.started, a.pane));
    }

    let mut placed: HashSet<PaneId> = HashSet::new();
    let mut out = Vec::new();
    let team = |lead: &Agent, placed: &mut HashSet<PaneId>| {
        placed.insert(lead.pane);
        let mut members = Vec::new();
        // Depth first, so a helper's own helpers come right after it.
        let mut stack: Vec<(&Agent, usize)> = children.get(&lead.pane).map(|k| k.iter().rev().map(|a| (*a, 1)).collect()).unwrap_or_default();
        while let Some((a, depth)) = stack.pop() {
            if !placed.insert(a.pane) {
                continue;
            }
            members.push(Member { agent: a.clone(), depth });
            if let Some(k) = children.get(&a.pane) {
                stack.extend(k.iter().rev().map(|c| (*c, depth + 1)));
            }
        }
        // A pipeline's own state is its runner's printing; its stages' is what matters (a
        // pipeline nested in a team too).
        let counts = |a: &Agent| a.kind != AgentKind::Pipeline || !children.contains_key(&a.pane);
        let status = members.iter().map(|m| &m.agent).filter(|a| counts(a)).map(|a| a.status).chain(counts(lead).then_some(lead.status)).min().unwrap_or(lead.status);
        Group { lead: Agent { status, ..lead.clone() }, members }
    };
    for a in agents.iter().filter(|a| parent(a).is_none()) {
        let g = team(a, &mut placed);
        out.push(g);
    }
    // Anything left was in a loop of parents (which nex-host never makes): on its own.
    for a in agents {
        if !placed.contains(&a.pane) {
            let g = team(a, &mut placed);
            out.push(g);
        }
    }
    out.sort_by_key(|g| (g.lead.status, g.lead.tab.unwrap_or(usize::MAX), g.lead.pane));
    out
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::agents::model::{Source, Status, Team};

    type Shape = Vec<(PaneId, Status, Vec<(PaneId, usize)>)>;

    fn agent(pane: PaneId, status: Status, parent: Option<PaneId>, started: u64) -> Agent {
        Agent {
            pane,
            tab: None,
            kind: AgentKind::Claude,
            name: format!("a{pane}"),
            mark: None,
            cwd: None,
            status,
            since: Instant::now(),
            source: Source::Local,
            team: Team { parent, role: None, started },
        }
    }

    fn shape(groups: &[Group]) -> Shape {
        groups.iter().map(|g| (g.key(), g.lead.status, g.members.iter().map(|m| (m.agent.pane, m.depth)).collect())).collect()
    }

    #[test]
    fn helpers_go_under_their_lead_in_the_order_they_started() {
        let agents = vec![
            agent(1, Status::Idle, None, 0),
            agent(2, Status::Idle, Some(1), 20),
            agent(3, Status::Idle, Some(1), 10),
            agent(4, Status::Idle, Some(3), 30),
            agent(5, Status::Idle, None, 0),
        ];
        assert_eq!(shape(&groups(&agents)), vec![(1, Status::Idle, vec![(3, 1), (4, 2), (2, 1)]), (5, Status::Idle, vec![])]);
        let g = &groups(&agents)[0];
        assert!(g.is_team() && !g.is_pipeline());
        assert_eq!(g.direct().map(|m| m.agent.pane).collect::<Vec<_>>(), vec![3, 2]);
    }

    #[test]
    fn a_team_takes_its_most_urgent_members_status_and_sorts_by_it() {
        let agents = vec![agent(9, Status::Working, None, 0), agent(1, Status::Idle, None, 0), agent(2, Status::NeedsYou, Some(1), 0)];
        let g = groups(&agents);
        assert_eq!(shape(&g), vec![(1, Status::NeedsYou, vec![(2, 1)]), (9, Status::Working, vec![])]);
    }

    #[test]
    fn a_pipelines_status_is_its_stages() {
        let mut pipe = agent(1, Status::Working, None, 0);
        pipe.kind = AgentKind::Pipeline;
        let g = groups(&[pipe.clone(), agent(2, Status::Idle, Some(1), 0)]);
        assert_eq!(g[0].lead.status, Status::Idle, "not the runner printing");
        assert!(g[0].is_pipeline());
        assert_eq!(groups(&[pipe])[0].lead.status, Status::Working, "on its own it's its own");
    }

    #[test]
    fn a_pipeline_an_agent_asked_for_is_in_its_card_as_steps() {
        let mut pipe = agent(2, Status::Working, Some(1), 10);
        pipe.kind = AgentKind::Pipeline;
        pipe.name = "feature".into();
        let stage = |pane, name: &str, role: &str, started| {
            let mut a = agent(pane, Status::Working, Some(2), started);
            a.name = name.into();
            a.team.role = Some(role.into());
            a
        };
        let agents = vec![
            agent(1, Status::Idle, None, 0),
            pipe,
            stage(3, "build", "build", 20),
            stage(4, "build-2", "build", 30),
            Agent { status: Status::Idle, ..stage(5, "merge", "merge", 40) },
            agent(6, Status::Idle, Some(1), 50),
        ];
        let g = groups(&agents);
        assert_eq!(g.len(), 1, "one card: the agent, its pipeline and its stages");
        assert_eq!(g[0].lead.status, Status::Working, "the stages' status counts");
        let all_idle: Vec<Agent> = agents.iter().map(|a| if a.kind == AgentKind::Pipeline { a.clone() } else { Agent { status: Status::Idle, ..a.clone() } }).collect();
        assert_eq!(groups(&all_idle)[0].lead.status, Status::Idle, "not the runner printing");
        assert_eq!(g[0].size(), "1 helper · 1 pipeline");

        let strip: Vec<String> = g[0]
            .strip()
            .iter()
            .map(|s| match s {
                Step::Heading(n) => format!("{n}:"),
                Step::Stage(a) => format!("stage {}", a.name),
                Step::Helper(a) => format!("helper {}", a.name),
            })
            .collect();
        assert_eq!(strip, ["feature:", "stage build", "stage build-2", "stage merge", "helper a6"]);
        assert!(same_step(&agents[2], &agents[3]) && !same_step(&agents[3], &agents[4]), "build and build-2 are one step");
    }

    #[test]
    fn helpers_whose_lead_is_gone_and_loops_stand_alone() {
        let agents = vec![agent(2, Status::Idle, Some(77), 0), agent(3, Status::Idle, Some(4), 0), agent(4, Status::Idle, Some(3), 0)];
        let g = groups(&agents);
        assert_eq!(g.len(), 2, "{:?}", shape(&g));
        assert_eq!(g[0].key(), 2);
        assert!(!g[0].is_team());
        assert_eq!(g[1].members.len(), 1, "the loop is broken once, not dropped");
    }
}
