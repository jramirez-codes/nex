//! The Agents pane's words: what a card, a helper row and a capsule say. A helper's tag
//! (`member_tag`: `Build-01`, from its role or session name and a number), the task its
//! terminal's title gives, the folder an agent works in, and a card's title and detail
//! lines (`header_text`; a pipeline's says how its run stands). Plain strings, no drawing.

use crate::agents::group::Group;
use crate::agents::model::{Agent, AgentKind, Source};

/// `text` with its first letter a capital: `build` → `Build`.
pub(super) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A session name without the number `nex-host` gives a second agent of the same name, and
/// that number: `build-2` → (`build`, 2).
pub(super) fn numbered(name: &str) -> (&str, Option<u32>) {
    match name.rsplit_once('-') {
        Some((stem, n)) if !stem.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (stem, n.parse().ok()),
        _ => (name, None),
    }
}

/// The name a helper's session goes by (its card's name while its title holds a task
/// instead, or the program when nothing else is known).
pub(super) fn session_name(a: &Agent) -> &str {
    match &a.session {
        Some(name) if !name.is_empty() => name,
        _ if a.task.is_none() => &a.name,
        _ => a.kind.process(),
    }
}

/// What kind of helper it is, as its capsule says and its tag starts: its role (or pipeline
/// stage), else its session's name without its number, capitalized (`Build`). A sub-agent's
/// is its type (`Explore`), else `Sub`.
pub(super) fn member_word(a: &Agent) -> String {
    let role = a.team.role.as_deref().map(str::trim).filter(|r| !r.is_empty());
    capitalized(match role {
        Some(role) => role,
        None if a.team.sub => "Sub",
        None => numbered(session_name(a)).0,
    })
}

/// What a helper goes by in its team: its word and a number (`Build-01`). The number is its
/// session name's (`build-2` is 02), else its place among the `siblings` (the helpers its
/// parent started, itself among them or not) with the same word, in the order they started.
/// (The view tags a whole team at once: [`member_tags`].)
#[cfg(test)]
pub(super) fn member_tag(a: &Agent, siblings: &[&Agent]) -> String {
    let words: Vec<String> = siblings.iter().map(|s| member_word(s)).collect();
    tag_among(a, &member_word(a), siblings.iter().copied().zip(words.iter().map(String::as_str)))
}

/// [`member_tag`] for `a`, whose word is `word`, among `siblings` with theirs.
fn tag_among<'a>(a: &Agent, word: &str, siblings: impl Iterator<Item = (&'a Agent, &'a str)>) -> String {
    let own = if a.team.sub { None } else { numbered(session_name(a)).1 };
    let n = own.unwrap_or_else(|| {
        let order = |x: &Agent| (x.team.started, x.pane);
        let before = siblings.filter(|(s, w)| s.pane != a.pane && order(s) < order(a) && *w == word).count();
        before as u32 + 1
    });
    format!("{word}-{n:02}")
}

/// What a helper is doing, for its row: the task in its terminal's title, live. A pipeline's
/// is how its run stands.
pub(super) fn member_task(a: &Agent) -> Option<&str> {
    let task = a.task.as_deref()?;
    let task = if a.kind == AgentKind::Pipeline { nex_host::proto::split_pipeline_title(task).0 } else { task.trim() };
    (!task.is_empty()).then_some(task)
}

/// Each member's tag, in `g.members`' order: [`member_tag`] among the helpers its parent
/// started, each helper's word worked out once.
pub(super) fn member_tags(g: &Group) -> Vec<String> {
    let words: Vec<String> = g.members.iter().map(|m| member_word(&m.agent)).collect();
    let all = || g.members.iter().map(|m| &*m.agent).zip(words.iter().map(String::as_str));
    all().map(|(a, word)| tag_among(a, word, all().filter(|(s, _)| s.team.parent == a.team.parent))).collect()
}

/// The last part of the folder an agent works in (`nex`, or a worktree's own folder), so
/// which checkout each one is in reads at a glance.
pub(super) fn folder(a: &Agent) -> Option<String> {
    let dir = a.cwd.as_ref()?;
    let name = dir.file_name().map_or_else(|| dir.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
    (!name.is_empty()).then_some(name)
}

/// The quiet line under a card's title: its folder (or, without one, what kind of agent it
/// is), a team's size, and its tab, or `Detached` for an agent on an ssh host no pane shows.
pub(super) fn card_detail(a: &Agent, team_size: Option<&str>) -> String {
    let mut parts = vec![folder(a).unwrap_or_else(|| a.kind.label().to_string())];
    parts.extend(team_size.map(str::to_string));
    match (a.tab, &a.source) {
        (Some(tab), _) => parts.push(format!("Tab {}", tab + 1)),
        (None, Source::Remote { pane: None, .. }) if team_size.is_none() => parts.push("Detached".into()),
        _ => {}
    }
    parts.join(" · ")
}

/// What a card's header says.
pub(super) struct HeaderText {
    /// In bold: the agent's task (or name).
    pub(super) title: String,
    /// After the title, lighter: how a pipeline's run stands (`2 building · 1 merged`).
    pub(super) counts: Option<String>,
    /// The quiet line under them.
    pub(super) detail: String,
}

/// A card's header: its agent's name over [`card_detail`]. A pipeline whose runner says how
/// it stands (its title; one from an older `nex-host` has none) is its name, capitalized,
/// then the counts, over the last thing that happened and then the detail.
pub(super) fn header_text(a: &Agent, team_size: Option<&str>) -> HeaderText {
    let detail = card_detail(a, team_size);
    let stands = a.task.as_deref().filter(|_| a.kind == AgentKind::Pipeline).map(nex_host::proto::split_pipeline_title);
    match stands {
        Some((counts, event)) if !counts.is_empty() || !event.is_empty() => HeaderText {
            title: capitalized(&a.name),
            counts: (!counts.is_empty()).then(|| counts.to_string()),
            detail: if event.is_empty() { detail } else { format!("{event} \u{00b7} {detail}") },
        },
        _ => HeaderText { title: a.name.clone(), counts: None, detail },
    }
}

/// The quiet line under a helper's tag and task: its folder (or, without one, what kind of
/// agent it is). A sub-agent's is its agent's folder, as a stage's is (its tag says its type).
pub(super) fn member_detail(a: &Agent) -> String {
    folder(a).unwrap_or_else(|| a.kind.label().to_string())
}
