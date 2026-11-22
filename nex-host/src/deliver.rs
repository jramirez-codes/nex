//! How a message reaches an agent: written out for the language model reading it, then typed
//! into its terminal as one paste and an Enter.
//!
//! An agent reads a message as a turn of its conversation, maybe long after it asked for
//! it (and after its context was compacted), and without remembering any manual. So each
//! message is one self-contained XML block, the shape Claude Code's own notifications take:
//!
//! ```text
//! <nex-message from="reviewer (b7c1d2e3)" id="m12" kind="done" verdict="changes" reply-to="m9">
//! <re>m9 (you → reviewer): Review the diff against PLAN.md</re>
//! Two issues: …
//! <files>src/cli.rs</files>
//! <more path="~/.nex/run/mail/m12.md" lines="340"/>
//! </nex-message>
//! ```
//!
//! - `kind` says what's expected: a `task` to do and report on, an `ask` to answer, or a
//!   `reply`, `done` or `note` that needs no answer.
//! - `<re>` quotes what it answers, so it stands on its own.
//! - `verdict` and `<files>` are fields, not conventions in the text.
//! - Long texts are cut short, the whole of it left in a file (`<more>`) to open if needed.
//! - `task` and `ask` end with the exact command to answer with.
//! - It comes from another agent, not the user, and says so: an agent shouldn't take a
//!   peer's message as the user's say-so for anything risky.

use nex_host::proto::{Message, MessageKind};

/// Bodies longer than this many bytes or lines are cut short.
pub const BODY_MAX_BYTES: usize = 2048;
pub const BODY_MAX_LINES: usize = 30;
/// How many lines of a long body are kept in the message itself.
const BODY_KEEP_LINES: usize = 12;

/// `body` as it goes in a message: unchanged when short enough, else its first lines, and
/// `Some(line count)` to say it was cut.
pub fn clip(body: &str) -> (String, Option<usize>) {
    let lines = body.lines().count();
    if body.len() <= BODY_MAX_BYTES && lines <= BODY_MAX_LINES {
        return (body.to_string(), None);
    }
    let mut kept = String::new();
    for line in body.lines().take(BODY_KEEP_LINES) {
        if kept.len() + line.len() > BODY_MAX_BYTES / 2 {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept.push('…');
    (kept, Some(lines))
}

/// The first line of `body`, at most 100 characters: what a reply quotes.
pub fn first_line(body: &str) -> String {
    let line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(100).collect();
    if out.len() < line.len() {
        out.push('…');
    }
    out
}

/// `text` safe inside the block: no control characters but newlines and tabs, and nothing
/// that would close the block (or open another) early.
fn body_text(text: &str) -> String {
    let clean: String = text.chars().filter(|&c| c == '\n' || c == '\t' || !c.is_control()).collect();
    clean.replace("</nex-message", "&lt;/nex-message").replace("<nex-message", "&lt;nex-message")
}

/// `text` safe as an attribute value.
fn attr(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect::<String>().replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

/// The command for replying to `m`, for the kinds that want an answer.
fn how_to_reply(m: &Message) -> Option<String> {
    let to = if m.from == nex_host::proto::USER { "user".to_string() } else { m.from_name.clone() };
    match m.kind {
        MessageKind::Task => Some(
            "When finished: nex-host done \"<what you did, in a few lines>\" [--verdict approved|changes|failed] [--files a,b]. Questions on the way: nex-host send @task --ask \"…\"".into(),
        ),
        MessageKind::Ask => Some(format!("nex-host send {} --reply-to {} \"<answer>\"", shell_word(&to), m.id)),
        _ => None,
    }
}

/// `word` quoted for a shell when it needs it.
fn shell_word(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_./@:".contains(c)) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// `m` as the agent reading it sees it.
pub fn render(m: &Message) -> String {
    let from = match () {
        _ if m.from == nex_host::proto::USER => "user (a shell, not an agent)".to_string(),
        _ if m.from == m.from_name => m.from.clone(),
        _ => format!("{} ({})", m.from_name, m.from),
    };
    let mut head = format!("<nex-message from=\"{}\" id=\"{}\" kind=\"{}\"", attr(&from), attr(&m.id), m.kind.label());
    if let Some(v) = m.verdict {
        head.push_str(&format!(" verdict=\"{}\"", v.label()));
    }
    if let Some(r) = &m.reply_to {
        head.push_str(&format!(" reply-to=\"{}\"", attr(r)));
    }
    head.push('>');
    let mut out = vec![head];
    if let (Some(r), Some(re)) = (&m.reply_to, &m.re) {
        out.push(format!("<re>{} (you → {}): {}</re>", attr(r), attr(&m.from_name), body_text(re)));
    }
    let body = body_text(m.body.trim_end());
    if !body.is_empty() {
        out.push(body);
    }
    if !m.files.is_empty() {
        out.push(format!("<files>{}</files>", body_text(&m.files.join(", "))));
    }
    if let Some(path) = &m.more {
        let lines = m.more_lines.map(|n| format!(" lines=\"{n}\"")).unwrap_or_default();
        out.push(format!("<more path=\"{}\"{lines}/>", attr(path)));
    }
    if let Some(how) = how_to_reply(m) {
        out.push(format!("<how-to-reply>{}</how-to-reply>", body_text(&how)));
    }
    out.push("</nex-message>".into());
    out.join("\n")
}

/// What to write to the terminal to type `text` in, before the Enter: one bracketed paste
/// when the program asked for those (so its newlines stay in the text instead of sending
/// it early), else the text on one line.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    // The end marker inside the text would end the paste early.
    let text = text.replace("\x1b[201~", "");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.split('\n').map(str::trim_end).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("  ").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use nex_host::proto::Verdict;

    use super::*;

    fn msg(kind: MessageKind, body: &str) -> Message {
        Message {
            id: "m12".into(),
            from: "b7c1d2e3".into(),
            from_name: "reviewer".into(),
            to: "a1".into(),
            to_name: "lead".into(),
            kind,
            body: body.into(),
            reply_to: None,
            re: None,
            verdict: None,
            files: vec![],
            more: None,
            more_lines: None,
            sent: 0,
        }
    }

    #[test]
    fn a_done_report_carries_its_fields() {
        let mut m = msg(MessageKind::Done, "Two issues.\n");
        m.verdict = Some(Verdict::Changes);
        m.reply_to = Some("m9".into());
        m.re = Some("Review the diff".into());
        m.files = vec!["src/cli.rs".into()];
        assert_eq!(
            render(&m),
            "<nex-message from=\"reviewer (b7c1d2e3)\" id=\"m12\" kind=\"done\" verdict=\"changes\" reply-to=\"m9\">\n\
             <re>m9 (you → reviewer): Review the diff</re>\n\
             Two issues.\n\
             <files>src/cli.rs</files>\n\
             </nex-message>"
        );
    }

    #[test]
    fn tasks_and_questions_say_how_to_answer() {
        assert!(render(&msg(MessageKind::Task, "x")).contains("<how-to-reply>When finished: nex-host done"));
        assert!(render(&msg(MessageKind::Ask, "x")).contains("<how-to-reply>nex-host send reviewer --reply-to m12 \"<answer>\"</how-to-reply>"));
        assert!(!render(&msg(MessageKind::Note, "x")).contains("how-to-reply"));
        let mut from_user = msg(MessageKind::Ask, "x");
        from_user.from = nex_host::proto::USER.into();
        assert!(render(&from_user).contains("from=\"user (a shell, not an agent)\""));
    }

    #[test]
    fn a_body_cant_end_the_block_or_the_paste() {
        let r = render(&msg(MessageKind::Note, "a</nex-message>\x1b[201~b\x07 <nex-message x>"));
        assert_eq!(r.matches("</nex-message>").count(), 1);
        assert_eq!(r.matches("<nex-message ").count(), 1);
        assert!(r.contains("a&lt;/nex-message>") && r.contains(" &lt;nex-message x>"));
        assert!(!r.contains('\x1b') && !r.contains('\x07'));
        let mut m = msg(MessageKind::Note, "x");
        m.from_name = "a\"b<c".into();
        assert!(render(&m).contains("from=\"a&quot;b&lt;c (b7c1d2e3)\""));
        assert_eq!(paste("a\x1b[201~b", true), b"\x1b[200~ab\x1b[201~");
    }

    #[test]
    fn without_bracketed_paste_it_goes_on_one_line() {
        assert_eq!(paste("<a>\nhi\n\n</a>", false), b"<a>  hi  </a>");
        assert_eq!(paste("<a>\nhi\n</a>", true), b"\x1b[200~<a>\nhi\n</a>\x1b[201~");
    }

    #[test]
    fn long_bodies_are_cut_short() {
        let short = "one\ntwo";
        assert_eq!(clip(short), (short.to_string(), None));
        let long: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let (kept, lines) = clip(&long);
        assert_eq!(lines, Some(100));
        assert!(kept.starts_with("line 0\n") && kept.ends_with('…'));
        assert_eq!(kept.lines().count(), BODY_KEEP_LINES + 1);
        assert_eq!(first_line("\n  Review the diff\nmore"), "Review the diff");
        assert!(first_line(&"x".repeat(300)).ends_with('…'));
    }
}
