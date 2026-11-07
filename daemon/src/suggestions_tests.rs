use super::*;

fn project_with_records(count: u32) -> Project {
    let mut project = Project {
        root: PathBuf::from("/tmp/repo"),
        next_record: count + 1,
        next_line: 1,
        ..Default::default()
    };
    for id in 1..=count {
        project.records.push(Record {
            id,
            session: format!("session-{id}"),
            turn: "2026-11-05T00:00:00Z".into(),
            kind: "searched_again".into(),
            text: "cargo test를 다시 돌림".into(),
            at: 1_000 + i64::from(id),
            fresh: true,
            signal: Signal::Improvement,
            said: None,
            given: None,
        });
        project
            .sessions
            .insert(format!("session-{id}"), 1_000 + i64::from(id));
    }
    project
}

fn doc_fix() -> Fix {
    Fix::Doc {
        title: "Rust 검증은 script/verify.sh로만".into(),
        path: "AGENTS.md".into(),
        draft: "cargo test 대신 script/verify.sh를 쓴다.".into(),
    }
}

#[test]
fn keeps_only_well_formed_improvements_masked_and_capped() {
    let output = "제목\n요약\n\
        개선: searched_again | cargo test를 세 번 다시 돌림\n\
        개선: no_doc | 배포 문서가 없음, 문의는 dev@example.com\n\
        개선: bogus | 버려짐\n\
        개선: wrong_guess 구분자 없음\n\
        개선: searched_again | cargo test를 세 번 다시 돌림\n\
        개선: tool_friction | /Users/someone/bin 경로가 PATH에 없음\n\
        개선: wrong_guess | 넷째라 버려짐";
    let kept = parse_improvements(output);
    assert_eq!(
        kept,
        vec![
            (
                "searched_again".into(),
                "cargo test를 세 번 다시 돌림".into()
            ),
            ("no_doc".into(), "배포 문서가 없음, 문의는 <email>".into()),
            ("tool_friction".into(), "~/bin 경로가 PATH에 없음".into()),
        ]
    );
}

#[test]
fn masks_tokens_and_assignments() {
    assert_eq!(
        mask("key sk-abcdefghijklmnopqrstu and token=abc123"),
        "key <secret> and token=<secret>"
    );
}

#[test]
fn counts_the_last_turns_tool_use_without_keeping_commands() {
    let jsonl = [
        r#"{"type":"user","timestamp":"t1","message":{"content":"first"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
        r#"{"type":"user","timestamp":"t2","message":{"content":"second"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}},{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}},{"type":"tool_use","name":"Read","input":{"file_path":"/a.rs"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"boom"}]}}"#,
    ]
    .join("\n");
    let LastTurn { turn, summary, .. } = last_turn(&jsonl).unwrap();
    assert_eq!(turn, "t2");
    assert!(summary.contains("tool calls 3"), "{summary}");
    assert!(summary.contains("distinct files read 1"), "{summary}");
    assert!(
        summary.contains("more than once 1 (most 2 times)"),
        "{summary}"
    );
    assert!(summary.contains("failed calls 1"), "{summary}");
    assert!(!summary.contains("cargo"), "{summary}");
}

#[test]
fn opens_attaches_and_strikes_lines_and_refuses_bad_fixes() {
    let mut project = project_with_records(3);
    assert_eq!(
        apply_edit(
            &mut project,
            Edit::Open {
                discomfort: "같은 곳에서 헤맴".into(),
                cause: String::new(),
                fix: Some(doc_fix()),
                records: vec![1, 99],
            }
        ),
        "opened s1"
    );
    assert_eq!(project.line(1).unwrap().records, vec![1]);
    assert_eq!(
        apply_edit(
            &mut project,
            Edit::Attach {
                line: 1,
                records: vec![2, 2, 3]
            }
        ),
        "attached 2 to s1"
    );
    let outside = Fix::Doc {
        title: "t".into(),
        path: "../escape.md".into(),
        draft: "d".into(),
    };
    assert!(
        apply_edit(
            &mut project,
            Edit::Open {
                discomfort: "x".into(),
                cause: String::new(),
                fix: Some(outside),
                records: vec![],
            }
        )
        .starts_with("open refused")
    );
    assert_eq!(
        apply_edit(
            &mut project,
            Edit::Strike {
                line: 1,
                why: "noise".into()
            }
        ),
        "struck s1"
    );
    assert_eq!(project.line(1).unwrap().stage, Stage::Struck);
}

#[test]
fn a_skipped_line_comes_back_once_when_its_evidence_doubles() {
    let mut project = project_with_records(6);
    apply_edit(
        &mut project,
        Edit::Open {
            discomfort: "헤맴".into(),
            cause: String::new(),
            fix: Some(doc_fix()),
            records: vec![1, 2],
        },
    );
    skip(project.line_mut(1).unwrap());
    apply_edit(
        &mut project,
        Edit::Attach {
            line: 1,
            records: vec![3],
        },
    );
    assert_eq!(project.line(1).unwrap().stage, Stage::Skipped);
    apply_edit(
        &mut project,
        Edit::Attach {
            line: 1,
            records: vec![4],
        },
    );
    let line = project.line(1).unwrap();
    assert_eq!(line.stage, Stage::New);
    assert!(line.revived);

    skip(project.line_mut(1).unwrap());
    apply_edit(
        &mut project,
        Edit::Attach {
            line: 1,
            records: vec![5, 6],
        },
    );
    assert_eq!(project.line(1).unwrap().stage, Stage::Skipped);
}

#[test]
fn a_kept_fix_is_effective_after_five_sessions_and_new_after_its_problem_returns() {
    let mut project = project_with_records(7);
    apply_edit(
        &mut project,
        Edit::Open {
            discomfort: "헤맴".into(),
            cause: String::new(),
            fix: Some(doc_fix()),
            records: vec![1],
        },
    );
    let line = project.line_mut(1).unwrap();
    line.stage = Stage::Applied;
    line.applied_at = Some(1_002);
    project.settle_watches();
    assert_eq!(project.line(1).unwrap().stage, Stage::Effective);

    apply_edit(
        &mut project,
        Edit::Attach {
            line: 1,
            records: vec![7],
        },
    );
    let line = project.line(1).unwrap();
    assert_eq!(line.stage, Stage::New);
    assert!(line.came_back);
}

#[test]
fn reads_refuse_paths_outside_the_repository_and_credentials() {
    for path in [
        "../x",
        "/etc/passwd",
        ".env",
        "config/.env.local",
        ".git/config",
        "keys/id_rsa",
        "a/secret.txt",
    ] {
        assert!(!is_repo_path(path), "{path}");
    }
    assert!(is_repo_path("AGENTS.md"));
    assert!(is_repo_path("docs/guide.md"));
}

#[test]
fn finds_the_repository_root_from_a_subfolder() {
    let dir = std::env::temp_dir().join(format!("naru-repo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let inner = dir.join("a/b");
    std::fs::create_dir_all(&inner).unwrap();
    assert_eq!(repo_root(&inner), Some(dir.clone()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reads_an_answer_wrapped_in_prose() {
    let answer = parse_answer(
        "Here you go:\n```json\n{\"edits\":[{\"op\":\"strike\",\"line\":2,\"why\":\"x\"}],\"reads\":[{\"tool\":\"git_log\",\"path\":null}],\"done\":true}\n```",
    )
    .unwrap();
    assert!(answer.done);
    assert_eq!(answer.reads, vec![ReadRequest::GitLog { path: None }]);
    assert_eq!(answer.edits.len(), 1);
}

#[test]
fn the_watch_counts_sessions_without_improvements_too() {
    let mut project = project_with_records(1);
    project.sessions = (0..5).map(|n| (format!("quiet-{n}"), 2_000)).collect();
    assert_eq!(project.sessions_since(1_500), 5);
    assert_eq!(project.sessions_since(2_000), 0);
}

#[test]
fn a_turn_marked_meta_does_not_start_a_turn() {
    let jsonl = [
        r#"{"type":"user","timestamp":"t1","message":{"content":"real"}}"#,
        r#"{"type":"user","isMeta":true,"timestamp":"t2","message":{"content":"caveat"}}"#,
    ]
    .join("\n");
    assert_eq!(last_turn(&jsonl).unwrap().turn, "t1");
}

#[test]
fn a_personal_repeat_groups_into_a_global_line_scored_from_its_second_saying() {
    let mut project = project_with_records(0);
    for (id, session) in [(1, "a"), (2, "a"), (3, "b")] {
        project.next_record = id + 1;
        project.records.push(Record {
            id,
            session: session.into(),
            turn: String::new(),
            signal: Signal::RepeatedRequest,
            kind: String::new(),
            text: "한국어로 답하기".into(),
            at: now(),
            fresh: true,
            said: Some(Said {
                personal: true,
                line: "답은 한국어로 해요".into(),
                excerpt: "한국어로".into(),
            }),
            given: None,
        });
    }
    group_by_tags(&mut project);
    assert_eq!(project.lines.len(), 1);
    let line = &project.lines[0];
    assert_eq!(
        line.fix,
        Some(Fix::Global {
            line: "답은 한국어로 해요".into()
        })
    );
    assert!((score(&project, line) - 3.).abs() < 0.01);
    raise(&mut project);
    assert!(!project.lines[0].held);
}

#[test]
fn lines_the_user_asked_for_take_no_room_from_raised_ones() {
    let mut project = project_with_records(3);
    project.settings.amount = Amount::Few;
    project.lines.push(Line {
        id: 1,
        fix: Some(doc_fix()),
        raised_at: Some(now()),
        asked: true,
        ..Default::default()
    });
    project.lines.push(Line {
        id: 2,
        fix: Some(doc_fix()),
        records: vec![1, 2, 3, 4],
        held: true,
        ..Default::default()
    });
    for id in 1..=3 {
        project.records[id - 1].at = now();
    }
    project.records.push(Record {
        session: "session-4".into(),
        id: 4,
        at: now(),
        ..project.records[0].clone()
    });
    raise(&mut project);
    assert!(!project.line(2).unwrap().held);
}
