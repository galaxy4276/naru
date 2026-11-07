use super::*;

fn capture(json: &str) -> Capture {
    serde_json::from_str(json).unwrap()
}

#[test]
fn a_read_adds_updates_and_merges_cards_and_drops_unknown_kinds() {
    let mut memory = empty(Path::new("/tmp/repo"));
    let given = HashMap::new();
    apply_capture(
        &mut memory,
        "s1",
        capture(
            r#"{"cards":[{"op":"add","kind":"rule","title":"검증은 script/verify.sh","body":"cargo test 대신 쓴다"},
                {"op":"add","kind":"bogus","title":"x","body":"y"},
                {"op":"add","kind":"fact","title":"포트 8044","body":"API 포트"}]}"#,
        ),
        &given,
        &[],
    );
    assert_eq!(memory.cards.len(), 2);
    assert_eq!(memory.cards[1].id, 2);
    apply_capture(
        &mut memory,
        "s2",
        capture(
            r#"{"cards":[{"op":"update","id":1,"body":"새 본문"},{"op":"merge","id":2,"into":1},{"op":"update","id":9,"body":"없음"}]}"#,
        ),
        &given,
        &[],
    );
    assert_eq!(memory.card(1).unwrap().body, "새 본문");
    assert_eq!(memory.card(1).unwrap().sources, vec!["s1", "s2"]);
    assert!(memory.card(2).unwrap().archived_at.is_some());

    let undo = memory
        .log
        .iter()
        .find(|change| change.what == "m1 고침")
        .unwrap()
        .id;
    memory.undo(undo);
    assert_eq!(memory.card(1).unwrap().body, "cargo test 대신 쓴다");
}

#[test]
fn signals_name_known_cards_and_ignored_ones_only_when_given() {
    let mut memory = empty(Path::new("/tmp/repo"));
    let given = HashMap::from([(1, "body".to_string())]);
    let signals = apply_capture(
        &mut memory,
        "s1",
        capture(
            r#"{"cards":[{"op":"add","kind":"preference","title":"답은 한국어로","body":"늘 한국어"}],
                "corrections":[{"card":1},{"card":7}],
                "ignored":[1,1],
                "repeats":[{"tag":"한국어로 답하기","personal":true,"line":"답은 한국어로 해요","excerpt":"한국어로 해","times":9}],
                "conflicts":[{"card":1,"doc":"AGENTS.md","says":"영어로"},{"card":1,"doc":"../x","says":"?"}]}"#,
        ),
        &given,
        &["AGENTS.md".to_string()],
    );
    let count = |signal| {
        signals
            .iter()
            .filter(|found| found.signal == signal)
            .count()
    };
    assert_eq!(count(Signal::Correction), 1);
    assert_eq!(count(Signal::MemoryIgnored), 1);
    assert_eq!(count(Signal::RepeatedRequest), MAX_REPEATS);
    assert_eq!(memory.conflicts.len(), 1);
}

#[test]
fn already_read_turns_are_marked_in_the_prompt() {
    let memory = empty(Path::new("/tmp/repo"));
    let talk = vec![
        (Some(100), "USER".to_string(), "예전 요청".to_string()),
        (Some(200), "USER".to_string(), "새 요청".to_string()),
    ];
    let text = prompt(&memory, &talk, Some(150), &HashMap::new(), &[], &[]);
    assert!(text.contains("USER (already read): 예전 요청"));
    assert!(text.contains("USER: 새 요청"));
    assert!(text.contains("STUDY LIST\n(none)"));
    let studying = ["Node.js 의 구조".to_string()];
    let text = prompt(&memory, &talk, Some(150), &HashMap::new(), &[], &studying);
    assert!(text.contains("STUDY LIST\nNode.js 의 구조"));
}

#[test]
fn a_read_without_learn_finds_parses_as_before() {
    let capture = parse(r#"{"cards":[]}"#).unwrap();
    assert!(capture.learn.is_empty());
    let capture =
        parse(r#"{"learn":[{"quote":"ALS가 뭐야","title":"ALS","why":"question"}]}"#).unwrap();
    assert_eq!(capture.learn[0].why, "question");
    let capture =
        parse(r#"{"cards":[{"op":"merge","id":1,"into":2}],"learn":[{"quote":"x","why":"odd"}]}"#)
            .unwrap();
    assert_eq!(capture.cards.len(), 1);
}

#[test]
fn only_a_session_given_the_body_holds_a_card_and_a_change_lifts_the_hold() {
    let mut memory = empty(Path::new("/tmp/repo"));
    apply_capture(
        &mut memory,
        "s1",
        capture(
            r#"{"cards":[{"op":"add","kind":"rule","title":"배포는 금요일 금지","body":"금요일엔 배포하지 않는다"}]}"#,
        ),
        &HashMap::new(),
        &[],
    );
    let receipt = |session: &str, how: &str| Receipt {
        session: session.into(),
        card: 1,
        how: how.into(),
        at: 1,
    };
    let receipts = [receipt("a", "index"), receipt("b", "body")];
    assert!(
        revision_to_hold(&memory, &receipts, 1, "a").is_err(),
        "a title in the index is not enough"
    );
    assert!(revision_to_hold(&memory, &receipts, 2, "b").is_err());
    let revision = revision_to_hold(&memory, &receipts, 1, "b").unwrap();
    let hold = Hold {
        revision,
        reason: "이제 금요일도 배포한다".into(),
        since: 1,
        session: "b".into(),
    };
    let holds = BTreeMap::from([(1, hold)]);
    assert_eq!(live_holds(holds.clone(), &memory).len(), 1);
    memory.cards[0].at += 1;
    assert!(
        live_holds(holds, &memory).is_empty(),
        "a card changed since is out of the hold"
    );
}
