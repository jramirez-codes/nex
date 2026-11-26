//! What a card shows that only changes when the list of agents does, worked out once per
//! change ([`Agents::generation`](crate::agents::model::Agents::generation)) rather than every
//! frame: its header's words, a team's capsule strip and their labels, each helper's tag,
//! folder and parent, the tree connectors, how a pipeline (and one nested in a team) stands,
//! and whether anything on it moves. A frame only reads it: what changes on its own (the
//! time in a state, what's on screen, the pointer) is worked out as it's drawn.
//!
//! To show something new on a card, work it out in [`Card::new`] and read it while drawing.

use std::collections::HashMap;
use std::sync::Arc;

use crate::agents::group::{Branch, Group, Step};
use crate::agents::model::{Agent, AgentKind, Status};
use crate::agents::view::paint::{pipeline_state, team_breathes};
use crate::agents::view::text::{HeaderText, header_text, member_detail, member_tags, member_word};
use crate::layout::PaneId;

/// How a pipeline stands from its stages ([`pipeline_state`]): the fraction done, the most
/// urgent status among them, and whether one is at work.
pub(super) type PipelineState = (f32, Status, bool);

/// A card's header, as drawn: its title, a pipeline's counts after it (with their separator),
/// and the quiet line under them.
pub(super) struct Header {
    pub(super) title: String,
    /// ` · 2 building · 1 merged`, after the title.
    pub(super) counts: Option<String>,
    pub(super) detail: String,
}

impl From<HeaderText> for Header {
    fn from(h: HeaderText) -> Header {
        Header { title: h.title, counts: h.counts.map(|c| format!(" \u{00b7} {c}")), detail: h.detail }
    }
}

/// One helper's words in an open team's list, and who started it.
pub(super) struct MemberText {
    /// `Build-01` ([`member_tag`](crate::agents::view::text::member_tag)).
    pub(super) tag: String,
    /// Its folder, or what kind of agent it is ([`member_detail`]).
    pub(super) detail: String,
    /// The agent that started it, when that's on the card.
    pub(super) parent: Option<Parent>,
}

/// Who started a helper: the card's lead, or another member (its index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Parent {
    Lead,
    Member(usize),
}

/// A pipeline among a team's helpers: its tile gets a ring and badge from its own stages.
pub(super) struct Nested {
    /// Its index in the team's members, and its stages'.
    pub(super) member: usize,
    pub(super) stages: Vec<usize>,
    pub(super) state: PipelineState,
}

/// Everything about one card that lasts until the list changes (see the module docs).
pub(super) struct Card {
    pub(super) group: Arc<Group>,
    pub(super) header: Header,
    /// A closed team's capsules ([`Group::strip`]) and what each says.
    pub(super) strip: Vec<Step>,
    pub(super) labels: Vec<String>,
    /// Each member's, in `group.members`' order.
    pub(super) members: Vec<MemberText>,
    /// Each member's connector ([`Group::branches`]).
    pub(super) branches: Vec<Option<Branch>>,
    /// A pipeline's own card: how its stages stand (`None` before it has any).
    pub(super) pipeline: Option<PipelineState>,
    pub(super) nested: Vec<Nested>,
    /// Whether something on the card moves while nothing changes (its dots breathe).
    pub(super) breathes: bool,
    /// The "Show All N" row's words, closed.
    pub(super) show_all: String,
    /// Each member's index, by its pane.
    index: HashMap<PaneId, usize>,
}

impl AsRef<Group> for Card {
    fn as_ref(&self) -> &Group {
        &self.group
    }
}

impl Card {
    pub(super) fn new(group: Arc<Group>) -> Card {
        let g = &*group;
        let team = g.is_team();
        let size = team.then(|| g.size());
        let index: HashMap<PaneId, usize> = g.members.iter().enumerate().map(|(i, m)| (m.agent.pane, i)).collect();
        let strip = if team { g.strip() } else { Vec::new() };
        let labels = strip.iter().map(|step| capsule_label(g, step)).collect();
        let members = g
            .members
            .iter()
            .zip(member_tags(g))
            .map(|(m, tag)| {
                let parent =
                    m.agent.team.parent.and_then(|p| if p == g.lead.pane { Some(Parent::Lead) } else { index.get(&p).copied().map(Parent::Member) });
                MemberText { tag, detail: member_detail(&m.agent), parent }
            })
            .collect();
        let pipeline =
            g.is_pipeline().then(|| pipeline_state(&g.direct().filter(|m| !m.agent.team.sub).map(|m| &*m.agent).collect::<Vec<_>>())).flatten();
        let nested = (0..g.members.len())
            .filter(|&i| g.members[i].agent.kind == AgentKind::Pipeline)
            .filter_map(|i| {
                let depth = g.members[i].depth;
                let stages: Vec<usize> = (i + 1..g.members.len())
                    .take_while(|&j| g.members[j].depth > depth)
                    .filter(|&j| g.members[j].depth == depth + 1 && !g.members[j].agent.team.sub)
                    .collect();
                let state = pipeline_state(&stages.iter().map(|&j| &*g.members[j].agent).collect::<Vec<_>>())?;
                Some(Nested { member: i, stages, state })
            })
            .collect();
        Card {
            header: header_text(&g.lead, size.as_deref()).into(),
            strip,
            labels,
            members,
            branches: g.branches(),
            pipeline,
            nested,
            breathes: if team { team_breathes(g) } else { g.lead.status == Status::Working },
            show_all: format!("Show All {}", g.members.len()),
            index,
            group,
        }
    }

    pub(super) fn key(&self) -> PaneId {
        self.group.key()
    }

    /// The index in `group.members` of the helper with card `pane`.
    pub(super) fn member_index(&self, pane: PaneId) -> Option<usize> {
        self.index.get(&pane).copied()
    }

    /// The agent that started member `i`, when it's on the card.
    pub(super) fn parent(&self, i: usize) -> Option<&Agent> {
        match self.members[i].parent? {
            Parent::Lead => Some(&self.group.lead),
            Parent::Member(j) => Some(&self.group.members[j].agent),
        }
    }
}

/// What a closed team's capsule says: a nested pipeline's name, else the helper's word, and
/// how many sub-agents it runs when it runs some (`Build ·2`).
fn capsule_label(g: &Group, step: &Step) -> String {
    match step {
        Step::Heading(name) => format!("{name}:"),
        Step::Stage(a) | Step::Helper(a) => match g.subs_of(a.pane) {
            0 => member_word(a),
            n => format!("{} \u{00b7}{n}", member_word(a)),
        },
    }
}

/// The cards the view keeps: `agents` as teams ([`groups`](crate::agents::group::groups)),
/// each with its [`Card`], and the generation of the list they were made from.
pub(super) struct Cards {
    pub(super) generation: u64,
    pub(super) cards: Arc<[Arc<Card>]>,
}

impl Cards {
    pub(super) fn new(agents: &crate::agents::model::Agents) -> Cards {
        let cards = crate::agents::group::groups(agents).into_iter().map(|g| Arc::new(Card::new(g))).collect();
        Cards { generation: agents.generation(), cards }
    }
}
