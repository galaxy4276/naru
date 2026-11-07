use super::*;

#[test]
fn resumes_only_plain_ids_through_plain_launchers() {
    assert_eq!(
        Agent::Claude.resume_command(Some("cl"), "0b5c-1"),
        Some("cl --resume 0b5c-1".into())
    );
    assert_eq!(
        Agent::Codex.resume_command(None, "abc"),
        Some("codex resume abc".into())
    );
    assert_eq!(Agent::Claude.resume_command(None, "a;rm -rf ~"), None);
    assert_eq!(Agent::Claude.resume_command(None, "--help"), None);
    assert_eq!(Agent::Claude.resume_command(Some("$(x)"), "abc"), None);
    assert_eq!(Agent::Gemini.resume_command(None, "latest"), None);
    assert_eq!(Agent::Amp.resume_command(None, "abc"), None);
}

#[test]
fn continue_and_picker_commands() {
    assert_eq!(
        Agent::Codex.continue_command(None),
        Some("codex resume --last".into())
    );
    assert_eq!(
        Agent::Claude.continue_command(Some("cl")),
        Some("cl --continue".into())
    );
    assert_eq!(
        Agent::Claude.session_picker_command(None),
        Some("claude --resume".into())
    );
    assert_eq!(Agent::Gemini.session_picker_command(None), None);
}

#[test]
fn codex_resume_keeps_only_known_permission_flags() {
    assert_eq!(
        Agent::Codex.resume_command_for_launch(None, "id1", Some("codex -s read-only -a never")),
        Some("codex resume id1 -s read-only -a never".into())
    );
    assert_eq!(
        Agent::Codex.resume_command_for_launch(None, "id1", Some("codex --no-daemon -s read-only")),
        Some("codex resume id1 --no-daemon -s read-only".into())
    );
    assert_eq!(
        Agent::Codex.resume_command_for_launch(None, "id1", Some("codex resume old --last")),
        Some("codex resume id1".into())
    );
    assert_eq!(
        Agent::Codex.resume_command_for_launch(None, "id1", Some("codex -s yolo")),
        None
    );
    assert_eq!(
        Agent::Codex.resume_command_for_launch(None, "id1", Some("codex --model x")),
        None
    );
    assert_eq!(
        Agent::Codex.resume_command_for_launch(None, "id1", None),
        None
    );
    assert_eq!(
        Agent::Claude.resume_command_for_launch(Some("cl"), "id1", None),
        Some("cl --resume id1".into())
    );
}

#[test]
fn detects_agents_from_typed_commands() {
    assert_eq!(
        Agent::detect("cl --model opus"),
        Some((Agent::Claude, Some("cl".into())))
    );
    assert_eq!(
        Agent::detect("FOO=1 /opt/homebrew/bin/codex"),
        Some((Agent::Codex, Some("/opt/homebrew/bin/codex".into())))
    );
    assert_eq!(Agent::detect("ls -la"), None);
    assert_eq!(Agent::detect("'unterminated"), None);
    assert_eq!(Agent::from_event_name("claude"), Some(Agent::Claude));
    assert_eq!(Agent::from_event_name("nope"), None);
}

#[test]
fn finds_conversations_that_began_with_clear() {
    let dir = std::env::temp_dir().join(format!("naru-clear-{}", std::process::id()));
    let project = dir.join("-Users-x-proj");
    std::fs::create_dir_all(&project).unwrap();
    let cleared = "11111111-1111-4111-8111-111111111111";
    let plain = "22222222-2222-4222-8222-222222222222";
    std::fs::write(
        project.join(format!("{cleared}.jsonl")),
        concat!(
            r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
            "\n"
        ),
    )
    .unwrap();
    std::fs::write(
        project.join(format!("{plain}.jsonl")),
        r#"{"type":"user","message":{"content":"hello"}}"#,
    )
    .unwrap();
    assert!(claude_cleared_into(&dir, cleared));
    assert!(!claude_cleared_into(&dir, plain));
    assert!(!claude_cleared_into(&dir, "../../etc/passwd"));
    std::fs::remove_dir_all(&dir).unwrap();
}
