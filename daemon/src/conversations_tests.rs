use super::*;

#[test]
fn indexes_typed_requests_and_finds_every_word() {
    let head = [
        r#"{"type":"user","cwd":"/Users/u/projects/naru","message":{"content":"<command-name>/clear</command-name>"}}"#,
        r#"{"type":"user","message":{"content":"학습함 패널을   옮겨 줘"}}"#,
        r#"{"type":"user","isMeta":true,"message":{"content":"hidden context"}}"#,
        r#"{"type":"assistant","message":{"content":"done"}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Brain도"},{"type":"tool_result","content":"x"}]}}"#,
    ]
    .join("\n");
    let conversation = parse_conversation("abc-1".into(), &head, "{\"trunc", 7).unwrap();
    assert_eq!(conversation.first, "학습함 패널을 옮겨 줘");
    assert_eq!(conversation.last, "Brain도");
    assert_eq!(conversation.cwd, PathBuf::from("/Users/u/projects/naru"));

    let older = Conversation {
        modified: 1,
        ..conversation.clone()
    };
    let found = search("naru 학습함", [older, conversation].into_iter());
    assert_eq!(found.iter().map(|c| c.modified).collect::<Vec<_>>(), [7, 1]);
    assert!(search("hidden", found.into_iter()).is_empty());
    assert!(
        parse_conversation(
            "x".into(),
            r#"{"type":"user","cwd":"/a","message":{"content":"<x>"}}"#,
            "",
            0
        )
        .is_none()
    );
}

#[test]
fn resumes_with_the_launcher_most_panes_use() {
    let session = |launcher: &str| {
        let mut session = crate::session::Session::new(Agent::Claude, Some(launcher.into()), None);
        session.agent = Agent::Claude;
        session
    };
    let sessions = [
        session("cl"),
        session("cl"),
        session("claude"),
        session("bad;rm"),
    ];
    assert_eq!(usual_launcher(sessions.iter()).as_deref(), Some("cl"));
    assert_eq!(usual_launcher([].iter()), None);
}
