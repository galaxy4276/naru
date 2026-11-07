use super::*;

#[test]
fn a_claude_turn_ends_interrupted_only_when_the_interruption_is_newest() {
    let prompt = r#"{"type":"user","message":{"role":"user","content":"fix the tests"}}"#;
    let answer = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"on it"}]}}"#;
    let esc = r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
    let side = r#"{"type":"assistant","isSidechain":true,"message":{"content":[]}}"#;

    assert!(claude_interrupted(&[prompt, answer, esc].join("\n")));
    assert!(claude_interrupted(&[prompt, esc, side].join("\n")));
    assert!(!claude_interrupted(&[esc, prompt].join("\n")));
    assert!(!claude_interrupted(&[esc, prompt, answer].join("\n")));
}

#[test]
fn a_codex_turn_ends_interrupted_when_it_was_aborted() {
    let started = r#"{"type":"event_msg","payload":{"type":"task_started"}}"#;
    let aborted = r#"{"type":"event_msg","payload":{"type":"turn_aborted"}}"#;
    let item = r#"{"type":"response_item","payload":{"type":"message"}}"#;
    assert!(codex_interrupted(
        &[started, item, aborted, item].join("\n")
    ));
    assert!(!codex_interrupted(&[aborted, started, item].join("\n")));
}

#[test]
fn a_finished_codex_turn_waits_on_an_open_plan_or_question() {
    let plan_turn = r#"{"type":"event_msg","payload":{"type":"task_started","collaboration_mode_kind":"plan"}}"#;
    let default_turn = r#"{"type":"event_msg","payload":{"type":"task_started","collaboration_mode_kind":"default"}}"#;
    let prompt = r#"{"type":"event_msg","payload":{"type":"user_message","message":"plan it"}}"#;
    let plan = r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"Plan","text":"1. x"}}}"#;
    let done = r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#;
    let ask = r#"{"type":"response_item","payload":{"type":"function_call","name":"request_user_input","call_id":"c1"}}"#;
    let answer =
        r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1"}}"#;
    let waits = |lines: &[&str]| codex_waits(&lines.join("\n"));
    assert_eq!(
        waits(&[plan_turn, prompt, plan, done]),
        Some("계획 승인 대기")
    );
    assert_eq!(waits(&[default_turn, prompt, plan, done]), None);
    assert_eq!(
        waits(&[default_turn, prompt, ask, done]),
        Some("질문 답 대기")
    );
    assert_eq!(waits(&[default_turn, prompt, ask, answer, done]), None);
}

#[test]
fn reads_claude_requests_with_their_last_replies() {
    let jsonl = [
        r#"{"type":"user","timestamp":"2026-11-06T01:00:00Z","message":{"content":"first?"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"which one?"}]}}"#,
        r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
        r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"the second"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}"#,
    ]
    .join("\n");
    let turns = turns(Agent::Claude, &jsonl);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].at.as_deref(), Some("2026-11-06T01:00:00Z"));
    assert_eq!(turns[0].reply.as_deref(), Some("which one?"));
    assert_eq!(turns[1].request, "the second");
    assert_eq!(turns[1].reply.as_deref(), Some("done"));
    assert!(turns[1].answers);
}

#[test]
fn reads_codex_requests_with_their_last_replies() {
    let jsonl = [
        r#"{"payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"text":"hi"}]}}}"#,
        r#"{"payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"text":"hello"}]}}}"#,
    ]
    .join("\n");
    let turns = turns(Agent::Codex, &jsonl);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].reply.as_deref(), Some("hello"));
}

#[test]
fn finds_transcripts_by_session_id() {
    let home = std::env::temp_dir().join(format!("naru-transcript-{}", Uuid::new_v4()));
    let claude = home.join(".claude/projects/-x");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("abc-1.jsonl"), "").unwrap();
    let id = Uuid::new_v4();
    let day = home.join(".codex/sessions/2026/11/06");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join(format!("rollout-2026-11-06T01-00-00-{id}.jsonl")),
        "",
    )
    .unwrap();

    assert!(find(Agent::Claude, "abc-1", &home).is_some());
    assert!(find(Agent::Claude, "../x", &home).is_none());
    if std::env::var_os("CODEX_HOME").is_none() {
        assert!(find(Agent::Codex, &id.to_string(), &home).is_some());
    }
    assert!(find(Agent::Codex, "not-a-uuid", &home).is_none());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn reads_a_tail_from_a_whole_line() {
    let path = std::env::temp_dir().join(format!("naru-tail-{}", Uuid::new_v4()));
    fs::write(&path, "aaaa\nbbbb\ncccc\n").unwrap();
    assert_eq!(read_tail(&path, 7).unwrap(), ("cccc\n".to_owned(), true));
    assert!(!read_tail(&path, 100).unwrap().1);
    fs::remove_file(path).unwrap();
}

#[test]
fn pending_agents_are_the_async_ones_without_a_notification() {
    let tail = [
        r#"{"type":"user","toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a1"}}"#,
        r#"{"type":"user","toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a2"}}"#,
        r#"{"type":"user","toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a3"}}"#,
        r#"{"type":"user","toolUseResult":{"backgroundTaskId":"b1"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"naru agent spawn --parent here --wait -- codex hi","run_in_background":true}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2"}]},"toolUseResult":{"backgroundTaskId":"b2"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"naru agent wait a4"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3"}]},"toolUseResult":{"backgroundTaskId":"b3"}}"#,
        r#"{"type":"user","isSidechain":true,"toolUseResult":{"isAsync":true,"agentId":"inner"}}"#,
        "{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"content\":\"<task-notification>\\n<task-id>a1</task-id>\\n<status>completed</status>\"}",
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<task-notification>\\n<task-id>a3</task-id>\"}}",
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<task-notification>\\n<task-id>b3</task-id>\"}}",
        "not json",
    ]
    .join("\n");
    assert_eq!(claude_pending_agents(&tail), 2, "a2 and the wait b2");
    assert_eq!(claude_pending_agents(""), 0);
    assert!(is_task_notification(
        "  <task-notification>\n<task-id>x</task-id>"
    ));
    assert!(!is_task_notification("please run <task-notification>"));
}
