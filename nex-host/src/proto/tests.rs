use crate::proto::session::TITLE_EVENT_MAX;
use crate::proto::*;
fn session() -> Session {
    Session {
        id: "a1b2c3d4".into(),
        kind: "claude".into(),
        name: "nex".into(),
        cwd: "/home/u/code/nex".into(),
        argv: vec!["claude".into()],
        pid: 42,
        started: 1_000,
        title: None,
        bell: false,
        attached: false,
        last_output: 0,
        last_input: 0,
        exit: None,
        claude: None,
        running: None,
        status: Status::Idle,
        parent: None,
        role: None,
        last_message: 0,
        subagents: Vec::new(),
    }
}

#[test]
fn protocol_2_sessions_still_parse() {
    let old = r#"{"id":"a","kind":"claude","name":"n","cwd":"/","argv":[],"pid":1,"started":0,"last_output":0,"last_input":0,"status":"idle"}"#;
    let s: Session = serde_json::from_str(old).unwrap();
    assert_eq!((s.parent, s.role, s.last_message), (None, None, 0));
    assert!(s.subagents.is_empty());
}

#[test]
fn message_requests_are_tagged_json() {
    let r: Request = serde_json::from_str(r#"{"op":"send","to":"@parent","kind":"done","body":"ok","verdict":"approved"}"#).unwrap();
    assert!(matches!(r, Request::Send { kind: MessageKind::Done, payload: Payload { verdict: Some(Verdict::Approved), .. }, force: false, .. }));
    let r: Request = serde_json::from_str(r#"{"op":"recv","from":"build","timeout_ms":5}"#).unwrap();
    assert!(matches!(r, Request::Recv { me: None, timeout_ms: Some(5), .. }));
    assert_eq!(MessageKind::parse("ask"), Some(MessageKind::Ask));
    assert_eq!(Verdict::parse("changes"), Some(Verdict::Changes));
}

#[test]
fn payloads_sit_at_the_top_level() {
    let payload = Payload {
        body: "ok".into(),
        reply_to: Some("m9".into()),
        verdict: Some(Verdict::Changes),
        files: vec!["a.rs".into()],
        pr: Some("42".into()),
    };
    let send = Request::Send { from: Some("a1".into()), to: "@task".into(), kind: MessageKind::Done, payload: payload.clone(), force: true };
    let line = r#"{"op":"send","from":"a1","to":"@task","kind":"done","body":"ok","reply_to":"m9","verdict":"changes","files":["a.rs"],"pr":"42","force":true}"#;
    assert_eq!(serde_json::to_string(&send).unwrap(), line);
    assert_eq!(serde_json::from_str::<Request>(line).unwrap(), send);
    let bare = r#"{"op":"send","to":"lead","kind":"note","body":"hi"}"#;
    let r: Request = serde_json::from_str(bare).unwrap();
    assert_eq!(r, Request::Send { from: None, to: "lead".into(), kind: MessageKind::Note, payload: Payload::text("hi"), force: false });
    assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"op":"send","to":"lead","kind":"note","body":"hi","force":false}"#);

    let m = Message {
        id: "m12".into(),
        from: "b7".into(),
        from_name: "reviewer".into(),
        to: "a1".into(),
        to_name: "lead".into(),
        kind: MessageKind::Done,
        payload,
        re: Some("Review it".into()),
        more: Some("~/.nex/run/mail/m12.md".into()),
        more_lines: Some(40),
        sent: 7,
    };
    let json = serde_json::to_value(&m).unwrap();
    let want = serde_json::json!({
        "id": "m12", "from": "b7", "from_name": "reviewer", "to": "a1", "to_name": "lead", "kind": "done",
        "body": "ok", "reply_to": "m9", "re": "Review it", "verdict": "changes", "files": ["a.rs"], "pr": "42",
        "more": "~/.nex/run/mail/m12.md", "more_lines": 40, "sent": 7,
    });
    assert_eq!(json, want);
    assert_eq!(serde_json::from_value::<Message>(want).unwrap(), m);
    let old = r#"{"id":"m1","from":"user","from_name":"user","to":"a","to_name":"a","kind":"note","body":"x","sent":0}"#;
    assert_eq!(serde_json::from_str::<Message>(old).unwrap().payload, Payload::text("x"));
}

#[test]
fn requests_are_tagged_json() {
    let r = Request::Attach { id: "x".into(), cols: 80, rows: 24 };
    let line = serde_json::to_string(&r).unwrap();
    assert_eq!(line, r#"{"op":"attach","id":"x","cols":80,"rows":24}"#);
    assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), r);
    assert_eq!(serde_json::from_str::<Request>(r#"{"op":"list"}"#).unwrap(), Request::List);
    let launch: Request = serde_json::from_str(r#"{"op":"launch","argv":["claude"]}"#).unwrap();
    assert!(matches!(launch, Request::Launch { cwd: None, .. }));
    let launch: Request = serde_json::from_str(r#"{"op":"launch","argv":[],"kind":"shell"}"#).unwrap();
    assert!(matches!(launch, Request::Launch { ref argv, .. } if argv.is_empty()));
    assert!(matches!(launch, Request::Launch { cwd: None, parent: None, .. }));
    let launch =
        Request::Launch { argv: vec!["x".into()], kind: None, name: None, cwd: None, cols: None, rows: None, env: None, parent: Some("a1".into()) };
    let line = serde_json::to_string(&launch).unwrap();
    assert!(line.contains(r#""parent":"a1""#), "{line}");
    assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), launch);
}

#[test]
fn a_pipelines_title_is_its_counts_then_its_last_event() {
    let t = pipeline_title("2 building · 1 in review", "\n✓ build (PR 7) approved:\tmerged\n");
    assert_eq!(t, "2 building · 1 in review — ✓ build (PR 7) approved: merged");
    assert_eq!(split_pipeline_title(&t), ("2 building · 1 in review", "✓ build (PR 7) approved: merged"));
    // Nothing said yet: the counts alone. An event that has a dash of its own splits once.
    assert_eq!(pipeline_title("1 building", "  "), "1 building");
    assert_eq!(split_pipeline_title("1 building"), ("1 building", ""));
    assert_eq!(split_pipeline_title("1 merged — ✓ done — really"), ("1 merged", "✓ done — really"));
    assert_eq!(split_pipeline_title(&pipeline_title("", "✗ stopped")), ("", "✗ stopped"));
    let long = pipeline_title("1 building", &"é".repeat(500));
    assert_eq!(split_pipeline_title(&long).1.chars().count(), TITLE_EVENT_MAX + 1, "cut, with an ellipsis");
    // The ellipsis is for what was cut: not for control characters left out, nor for
    // an event that just fits.
    assert_eq!(pipeline_title("1 building", "✓ \x1b[1mbuild\x07 \x1b done"), "1 building — ✓ [1mbuild done");
    let fits = "é".repeat(TITLE_EVENT_MAX);
    assert_eq!(split_pipeline_title(&pipeline_title("1 building", &fits)).1, fits);
    assert!(pipeline_title("1 building", &format!("{fits}\x07")).ends_with('é'));
    assert!(pipeline_title("1 building", &format!("\x07{fits}é")).ends_with("é…"));
}

#[test]
fn sessions_round_trip() {
    let mut s = session();
    s.claude = Some(Claude { status: Some("busy".into()), ..Claude::default() });
    let r = Response::sessions(vec![s]);
    let back: Response = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(back, r);
}

#[test]
fn status_prefers_exit_then_claude_then_bell_then_title_then_output() {
    let mut s = session();
    s.exit = Some(0);
    s.bell = true;
    assert_eq!(s.compute_status(10_000), Status::Exited);

    let mut s = session();
    s.claude = Some(Claude { status: Some("waiting_for_permission".into()), ..Claude::default() });
    assert_eq!(s.compute_status(10_000), Status::NeedsYou);
    s.claude = Some(Claude { status: Some("busy".into()), ..Claude::default() });
    assert_eq!(s.compute_status(10_000), Status::Working);
    s.bell = true;
    assert_eq!(s.compute_status(10_000), Status::NeedsYou);

    let mut s = session();
    s.title = Some("✳ Fix the tests".into());
    assert_eq!(s.compute_status(10_000), Status::Idle);
    s.title = Some("✻ Fix the tests".into());
    assert_eq!(s.compute_status(10_000), Status::Working);
    s.title = Some("Fix the tests".into());
    s.last_output = 9_500;
    assert_eq!(s.compute_status(10_000), Status::Working);
    s.last_input = 9_400;
    assert_eq!(s.compute_status(10_000), Status::Idle, "just the echo of typing");
    s.last_input = 0;
    assert_eq!(s.compute_status(20_000), Status::Idle);
}

#[test]
fn versions_compare_by_number_then_protocol() {
    assert!(older("nex-host 0.12.2 (protocol 3)", "nex-host 0.12.10 (protocol 3)"));
    assert!(older("nex-host 0.12.2 (protocol 3)", "nex-host 0.13.0-rc1 (protocol 3)"));
    assert!(older("nex-host 0.12.2 (protocol 2)", "nex-host 0.12.2 (protocol 3)"));
    assert!(!older("nex-host 0.12.2 (protocol 3)", "nex-host 0.12.2 (protocol 3)"));
    assert!(!older("nex-host 0.13.0 (protocol 3)", "nex-host 0.12.2 (protocol 3)"), "never a newer one");
    assert!(!older("something else", &version_line()));
    assert!(!older(&version_line(), "garbage"));
    assert_eq!(serde_json::from_str::<Request>(r#"{"op":"version"}"#).unwrap(), Request::Version);
}

#[test]
fn frames_carry_their_length() {
    assert_eq!(frame(FRAME_INPUT, b"hi"), vec![0, 0, 0, 0, 2, b'h', b'i']);
    assert_eq!(resize_frame(120, 40), vec![1, 0, 0, 0, 4, 0, 120, 0, 40]);
}
