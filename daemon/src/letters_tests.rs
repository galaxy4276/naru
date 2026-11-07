use super::*;
use uuid::Uuid;

fn record(from: Option<PaneId>, to: PaneId, at: u64) -> Record {
    Record {
        id: 0,
        from,
        to,
        from_name: "a1".into(),
        to_name: "a2".into(),
        from_title: String::new(),
        to_title: String::new(),
        kind: Kind::Request,
        text: "hi".into(),
        at,
        reply_to: None,
        state: State::Held,
        why: None,
        state_at: at,
    }
}

#[test]
fn numbers_letters_and_links_replies() {
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    let now = unix_now();
    let mut letters = Letters::new(4, Vec::new());
    assert_eq!(letters.add(record(Some(a), b, now)), 5);
    let mut answer = record(Some(b), a, now);
    answer.reply_to = Some(5);
    assert_eq!(letters.add(answer), 6);
    let views = serde_json::to_value(letters.views()).unwrap();
    assert_eq!(views[0]["answered_by"], 6);
    assert_eq!(views[1]["reply_to"], 5);
    assert_eq!(views[0]["kind"], "request");

    assert!(letters.set(5, State::Dropped, Some("gone")));
    assert!(!letters.set(5, State::Dropped, Some("gone")), "no change");
    assert!(!letters.set(99, State::Taken, None), "not on record");
    assert_eq!(letters.get(5).unwrap().why.as_deref(), Some("gone"));
}

#[test]
fn keeps_a_week_and_two_hundred_a_pair() {
    let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let now = unix_now();
    let mut old = record(Some(a), b, now - 8 * 24 * 60 * 60);
    old.id = 1;
    let mut letters = Letters::new(0, vec![old]);
    assert!(letters.all().is_empty(), "older than a week");
    assert_eq!(letters.next(), 1, "numbers are not reused");

    for _ in 0..PER_PAIR {
        letters.add(record(Some(a), b, now));
        letters.add(record(Some(b), a, now));
    }
    letters.add(record(None, c, now));
    let between = |x, y| {
        letters
            .all()
            .iter()
            .filter(|r| r.from == Some(x) && r.to == y || r.from == Some(y) && r.to == x)
            .count()
    };
    assert_eq!(between(a, b), PER_PAIR);
    assert_eq!(
        letters.all().first().unwrap().id,
        PER_PAIR as u32 + 2,
        "newest kept"
    );
    assert!(letters.all().iter().any(|r| r.from.is_none()));
}

#[test]
fn reads_what_it_saved() {
    let dir = std::env::temp_dir().join(format!("naru-letters-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let pane = Uuid::new_v4();
    let saved = serde_json::json!({
        "next_agent": 3,
        "agents": { pane.to_string(): 3 },
        "next_letter": 1,
        "letters": [record(None, pane, unix_now())],
    });
    save(dir.as_path(), &serde_json::to_vec(&saved).unwrap());
    let loaded = load(dir.as_path());
    assert_eq!(loaded.agents[&pane], 3);
    assert_eq!(loaded.letters.len(), 1);

    std::fs::write(dir.as_path().join("team.json"), "{").unwrap();
    assert!(load(dir.as_path()).letters.is_empty());
    assert!(dir.join("team.json.bad").exists(), "kept aside");
    std::fs::remove_dir_all(&dir).unwrap();
}
