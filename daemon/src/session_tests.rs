use super::*;

fn event(body: &str) -> Event {
    Event::parse(SENTINEL, body).expect("valid event")
}

fn none() -> &'static Path {
    Path::new("/nonexistent")
}

#[test]
fn parses_only_sentinel_v1_events_from_known_agents() {
    let parsed = event(
        r#"{"v":1,"agent":"claude","event":"permission_request","session_id":"s1","summary":"Run ls","tool_input":{"command":"ls"}}"#,
    );
    assert_eq!(parsed.agent, Agent::Claude);
    assert_eq!(parsed.kind, EventKind::PermissionRequest);
    assert_eq!(parsed.tool_input_preview.as_deref(), Some("ls"));
    assert!(Event::parse("other", r#"{"agent":"claude","event":"stop"}"#).is_none());
    assert!(Event::parse(SENTINEL, r#"{"v":2,"agent":"claude","event":"stop"}"#).is_none());
    assert!(Event::parse(SENTINEL, r#"{"agent":"nope","event":"stop"}"#).is_none());
}

#[test]
fn a_turn_runs_blocks_and_finishes() {
    let mut session = Session::new(Agent::Claude, Some("cl".into()), None);
    assert!(!session.shows_done());
    assert!(session.apply(
        &event(r#"{"agent":"claude","event":"prompt_submit","session_id":"s1","query":"hi"}"#),
        none()
    ));
    assert_eq!(session.status, Status::InProgress);
    assert_eq!(session.session_id.as_deref(), Some("s1"));
    session.apply(
        &event(r#"{"agent":"claude","event":"permission_request","session_id":"s1","summary":"Run ls"}"#),
        none(),
    );
    assert_eq!(
        session.status,
        Status::Blocked {
            message: Some("Run ls".into())
        }
    );
    session.apply(
        &event(r#"{"agent":"claude","event":"tool_complete","session_id":"s1"}"#),
        none(),
    );
    assert_eq!(session.status, Status::InProgress);
    assert_eq!(session.summary, None);
    session.apply(
        &event(
            r#"{"agent":"claude","event":"stop","session_id":"s1","query":"hi","response":"yo"}"#,
        ),
        none(),
    );
    assert_eq!(session.status, Status::Success);
    assert!(session.shows_done());
    assert_eq!(session.resume_command().as_deref(), Some("cl --resume s1"));
}

#[test]
fn ignores_other_sessions_and_stale_stops() {
    let mut session = Session::new(Agent::Claude, None, None);
    session.apply(
        &event(r#"{"agent":"claude","event":"prompt_submit","session_id":"s1","query":"a"}"#),
        none(),
    );
    assert!(!session.apply(
        &event(r#"{"agent":"claude","event":"stop","session_id":"other","query":"a"}"#),
        none()
    ));
    assert!(!session.apply(
        &event(r#"{"agent":"claude","event":"stop","session_id":"s1","query":"older"}"#),
        none()
    ));
    assert!(!session.apply(
        &event(r#"{"agent":"codex","event":"stop","session_id":"s1"}"#),
        none()
    ));
    assert_eq!(session.status, Status::InProgress);
    // A resumed or cleared session announces its new ID.
    assert!(session.apply(
        &event(r#"{"agent":"claude","event":"session_start","session_id":"s2","session_start_source":"resume"}"#),
        none()
    ));
    assert_eq!(session.session_id.as_deref(), Some("s2"));
}

#[test]
fn a_restored_session_takes_the_id_claude_continues_under() {
    let mut session = Session::new(Agent::Claude, None, None);
    session.session_id = Some("old".into());
    assert!(session.apply(
        &event(r#"{"agent":"claude","event":"prompt_submit","session_id":"new","query":"q"}"#),
        none()
    ));
    assert_eq!(session.session_id.as_deref(), Some("new"));
}

#[test]
fn out_of_order_events_are_dropped() {
    let mut session = Session::new(Agent::Codex, None, None);
    session.apply(
        &event(r#"{"agent":"codex","event":"prompt_submit","session_id":"s","turn_id":"t2","event_sequence":5}"#),
        none(),
    );
    assert!(!session.apply(
        &event(
            r#"{"agent":"codex","event":"stop","session_id":"s","turn_id":"t1","event_sequence":3}"#
        ),
        none()
    ));
    assert_eq!(session.status, Status::InProgress);
}

#[test]
fn idle_prompt_ends_an_interrupted_turn_and_session_ids_must_be_plain() {
    let mut session = Session::new(Agent::Claude, None, None);
    session.apply(
        &event(r#"{"agent":"claude","event":"prompt_submit","session_id":"a b"}"#),
        none(),
    );
    assert_eq!(session.session_id, None);
    session.apply(
        &event(r#"{"agent":"claude","event":"idle_prompt"}"#),
        none(),
    );
    assert_eq!(session.status, Status::Success);
    assert_eq!(session.resume_command(), None);
    assert_eq!(
        session.fallback_command().as_deref(),
        Some("claude --resume")
    );
}

#[test]
fn a_new_status_is_unread_and_dated() {
    let mut session = Session::new(Agent::Claude, None, None);
    assert!(session.read);
    session.since = None;
    session.apply(
        &event(r#"{"agent":"claude","event":"stop","session_id":"s1"}"#),
        none(),
    );
    assert_eq!(session.status, Status::Success);
    assert!(
        session.read,
        "a status that did not change keeps its read mark"
    );
    session.apply(
        &event(r#"{"agent":"claude","event":"prompt_submit","session_id":"s1"}"#),
        none(),
    );
    assert!(!session.read);
    assert!(session.since.is_some());
}

#[test]
fn a_turn_that_leaves_subagents_running_is_not_done_until_they_report() {
    let mut session = Session::new(Agent::Claude, None, None);
    session.apply(
        &event(
            r#"{"agent":"claude","event":"prompt_submit","session_id":"s1","query":"review it"}"#,
        ),
        none(),
    );
    let mut stop = event(
        r#"{"agent":"claude","event":"stop","session_id":"s1","query":"review it","response":"launched"}"#,
    );
    stop.pending_agents = 2;
    session.apply(&stop, none());
    assert_eq!(
        (session.status.clone(), session.subagents),
        (Status::InProgress, 2)
    );
    assert!(!session.shows_done());
    session.apply(
        &event(r#"{"agent":"claude","event":"idle_prompt","session_id":"s1"}"#),
        none(),
    );
    assert_eq!(
        session.status,
        Status::InProgress,
        "idling while subagents work is no end"
    );
    session.apply(
        &event(r#"{"agent":"claude","event":"tool_complete","session_id":"s1"}"#),
        none(),
    );
    let mut woke = event(
        r#"{"agent":"claude","event":"stop","session_id":"s1","query":"<task-notification>\n<task-id>a1</task-id>","response":"one back"}"#,
    );
    woke.pending_agents = 1;
    session.apply(&woke, none());
    assert_eq!(
        (session.status.clone(), session.subagents),
        (Status::InProgress, 1)
    );
    let done = event(
        r#"{"agent":"claude","event":"stop","session_id":"s1","query":"<task-notification>\n<task-id>a2</task-id>","response":"all done"}"#,
    );
    assert!(
        session.apply(&done, none()),
        "a turn a task result started is no stale stop"
    );
    assert_eq!(
        (session.status.clone(), session.subagents),
        (Status::Success, 0)
    );
    assert_eq!(
        session.query.as_deref(),
        Some("review it"),
        "the user's request stays on show"
    );
    assert_eq!(session.response.as_deref(), Some("all done"));
}
