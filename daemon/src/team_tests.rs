use super::*;
use uuid::Uuid;

#[test]
fn a_new_run_gets_a_new_id_unless_it_woke() {
    let mut team = Team::default();
    let pane = Uuid::new_v4();
    assert_eq!(team.id(pane), "a1");
    assert!(!team.start_run(pane), "the first run keeps the spawn id");
    assert_eq!(team.id(pane), "a1");
    assert!(!team.start_run(pane));
    assert_eq!(team.id(pane), "a2");
    assert_eq!(team.pane_for("a2"), Some(pane));
    team.asleep.insert(pane, "claude --resume x".into());
    assert!(team.start_run(pane));
    assert_eq!(team.id(pane), "a2");
    assert_eq!(team.pane_for("b2"), None);
}

#[test]
fn writes_letters_the_recipient_can_answer() {
    let typed = |id, from: Option<(&str, &str)>, kind, text: &str, reply_to| Typed {
        id,
        from: from.map(|(id, title)| (id.to_owned(), title.to_owned())),
        kind,
        text: text.to_owned(),
        reply_to,
    };
    let text = letter_text(&[
        typed(Some(7), Some(("a1", "fix")), Kind::Block, "which db?", None),
        typed(Some(9), Some(("a2", "")), Kind::Report, "done", Some(7)),
        typed(None, Some(("a3", "kid")), Kind::Watch, "stalled", None),
        typed(Some(10), None, Kind::Request, "look", None),
    ]);
    assert_eq!(
        text,
        "[Naru letter l7 from a1 \"fix\" · question]\nwhich db?\n(reply with: naru reply l7 \"<message>\")\n\n\
         [Naru letter l9 from a2 \"\" · report · reply to l7]\ndone\n(reply with: naru reply l9 \"<message>\")\n\n\
         [Naru letter from a3 \"kid\" · watch]\nstalled\n(reply with: naru tell a3 \"<message>\")\n\n\
         [Naru letter l10 from the user · request]\nlook"
    );
    assert_eq!(sanitize(" a\x1b[201~\x03b\n\tc "), "a[201~b\n\tc");
}

#[test]
fn keeps_ids_across_a_restart_and_gives_a_new_run_a_new_one() {
    let (kept, shell, closed) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut workspace = Workspace::default();
    workspace.panes.insert(kept, Default::default());
    workspace.panes.insert(shell, Default::default());
    let saved = letters::Saved {
        next_agent: 7,
        agents: [(kept, 3), (shell, 5), (closed, 7)].into(),
        ..Default::default()
    };
    let mut team = Team::default();
    team.restore(saved, &workspace);
    assert_eq!(team.id(kept), "a3", "a restored agent keeps its id");
    assert_eq!(team.pane_for("a7"), None, "closed panes are forgotten");
    assert!(!team.start_run(shell));
    assert_eq!(
        team.id(shell),
        "a8",
        "a new agent in a restored pane gets a new id"
    );
    let saved: letters::Saved = serde_json::from_slice(&team.take_saved().unwrap()).unwrap();
    assert_eq!(saved.agents[&shell], 8);
    assert_eq!(saved.next_agent, 8);
    assert!(team.take_saved().is_none(), "saved once per change");
}

#[test]
fn a_sleepers_letters_wait_from_when_it_wakes() {
    let mut team = Team::default();
    let pane = Uuid::new_v4();
    let long_ago = Instant::now() - Duration::from_secs(2 * 60 * 60);
    team.asleep.insert(pane, "claude --resume x".into());
    team.waking.insert(pane, long_ago);
    team.pending.push(Letter {
        id: Some(1),
        from: None,
        to: pane,
        kind: Kind::Report,
        text: "done".into(),
        now: false,
        at: long_ago,
    });
    assert!(team.start_run(pane));
    assert!(team.pending[0].at.elapsed() < Duration::from_secs(1));
    assert!(team.waking.is_empty());
}

#[test]
fn raises_a_question_the_parent_left_alone() {
    let mut team = Team::default();
    let child = Uuid::new_v4();
    let mut workspace = Workspace::default();
    workspace.panes.insert(child, Default::default());
    let past = Instant::now() - Duration::from_secs(120);
    team.questions.insert(child, ("which db?".into(), past));
    let view = team.view(&workspace);
    assert_eq!(view[&child].question.as_deref(), Some("which db?"));
    assert_eq!(
        view[&child].alert.as_deref(),
        Some("질문 · 부모가 2분째 답하지 않음")
    );
    team.reported.insert(child);
    assert_eq!(team.view(&workspace)[&child].alert, None);
    team.heard(child, true);
    assert!(
        team.view(&workspace)
            .get(&child)
            .is_none_or(|v| v.question.is_none())
    );
}

#[test]
fn accepts_only_known_keys() {
    assert_eq!(key_bytes("ctrl-c"), Some(&b"\x03"[..]));
    assert_eq!(key_bytes("f1"), None);
}
