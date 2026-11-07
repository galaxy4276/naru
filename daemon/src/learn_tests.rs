use super::*;

#[test]
fn numbers_marks_past_any_kept_id() {
    let mut store: Store = serde_json::from_str(r#"{"next_id":0,"marks":[]}"#).unwrap();
    let mark = |title: &str| Mark {
        id: 0,
        title: title.into(),
        hard: String::new(),
        excerpt: "x".into(),
        project: None,
        cwd: None,
        from_title: String::new(),
        source: None,
        remote: false,
        at: 0,
        stage: Stage::New,
        study_session: None,
        note: None,
    };
    assert_eq!(store.add(mark("a")), 1);
    store.marks[0].id = 7;
    assert_eq!(store.add(mark("b")), 8);
    let dir = std::env::temp_dir().join(format!("naru-learn-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("learn.json");
    store.save(&path).unwrap();
    assert_eq!(Store::load(&path).unwrap(), store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn takes_only_markdown_notes_inside_the_study_folder() {
    let dir = std::env::temp_dir().join(format!("naru-study-{}", uuid::Uuid::new_v4()));
    let root = dir.join("study");
    std::fs::create_dir_all(root.join("2026-11-06")).unwrap();
    let note = root.join("2026-11-06/x.md");
    std::fs::write(&note, "x").unwrap();
    std::fs::write(dir.join("outside.md"), "x").unwrap();
    std::fs::write(root.join("x.txt"), "x").unwrap();
    assert!(study_note(&note, &root).is_ok());
    assert!(study_note(&root.join("2026-11-06/../../outside.md"), &root).is_err());
    assert!(study_note(&root.join("x.txt"), &root).is_err());
    assert!(study_note(&root.join("missing.md"), &root).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn study_ids_and_note_links_cannot_break_out() {
    assert!(is_session_id("6f1c2a3e-1b2c-4d5e-8f90-123456789abc"));
    assert!(!is_session_id("abc; rm -rf ~"));
    assert!(!is_session_id("--resume"));
    assert_eq!(
        percent_encode("/study/a b/한&x.md"),
        "%2Fstudy%2Fa%20b%2F%ED%95%9C%26x.md"
    );
    assert_eq!(first_line("\n  첫 줄 \n둘째"), "첫 줄");
    assert_eq!(clip_bytes("가나", 4), "가\n…(잘림)");
}

#[test]
fn only_files_right_inside_a_numbered_material_folder_are_materials() {
    let dir = std::env::temp_dir().join(format!("naru-material-{}", uuid::Uuid::new_v4()));
    let learn = dir.join("learn");
    std::fs::create_dir_all(learn.join("3/deeper")).unwrap();
    std::fs::create_dir_all(learn.join("x")).unwrap();
    for file in ["3/a.html", "3/deeper/b.html", "x/c.html", "3.md"] {
        std::fs::write(learn.join(file), "x").unwrap();
    }
    std::fs::write(dir.join("outside.html"), "x").unwrap();
    std::os::unix::fs::symlink(dir.join("outside.html"), learn.join("3/link.html")).unwrap();
    assert_eq!(material_of(&learn.join("3/a.html"), &learn), Some(3));
    assert_eq!(material_of(&learn.join("3/deeper/b.html"), &learn), None);
    assert_eq!(material_of(&learn.join("x/c.html"), &learn), None);
    assert_eq!(material_of(&learn.join("3.md"), &learn), None);
    assert_eq!(
        material_of(&learn.join("3/../../outside.html"), &learn),
        None
    );
    assert_eq!(material_of(&learn.join("3/link.html"), &learn), None);
    assert_eq!(material_of(&learn.join("3/missing.html"), &learn), None);
    std::fs::write(learn.join("3/b.svg"), "x").unwrap();
    assert_eq!(materials_in(&learn.join("3")), ["a.html", "b.svg"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pages_that_load_files_by_relative_address_go_to_the_browser() {
    assert!(loads_relative_files(r#"<script src="app.js"></script>"#));
    assert!(loads_relative_files("<img\nSRC='./a.png'>"));
    assert!(loads_relative_files(r#"<a href="other.html">x</a>"#));
    assert!(!loads_relative_files(
        r##"<a href="#top">x</a><svg><use href="#g"/></svg>"##
    ));
    assert!(!loads_relative_files(
        r#"<img src="data:image/png;base64,AA">"#
    ));
    assert!(!loads_relative_files(
        r#"<link href="https://fonts.example/x.css">"#
    ));
    assert!(!loads_relative_files(
        r#"<div data-src="x.png"></div><img src="">"#
    ));
}

#[test]
fn a_selection_finds_its_paragraphs_in_the_conversation() {
    let reply = "앞 단락.\n\n컨테이너에는 **한도가 두 개** 따로 있습니다. 하나는 전체 한도\n1536MB입니다.\n\n뒤 단락.";
    let screen = "  컨테이너에는 한도가 두 개 따로 있습니다. 하나는\n  전체 한도 1536MB입니다.";
    assert_eq!(
        original_paragraphs(screen, &["다른 답", reply]).as_deref(),
        Some("컨테이너에는 **한도가 두 개** 따로 있습니다. 하나는 전체 한도\n1536MB입니다.")
    );
    assert_eq!(original_paragraphs("한도", &[reply]), None);
    assert_eq!(
        original_paragraphs("어디에도 없는 아주 긴 문장입니다", &[reply]),
        None
    );
}

fn turn(at: &str, request: &str, reply: Option<&str>) -> Turn {
    Turn {
        at: Some(at.into()),
        request: request.into(),
        reply: reply.map(Into::into),
        answers: false,
    }
}

#[test]
fn only_words_the_user_wrote_in_unread_turns_become_proposals() {
    let turns = [
        turn(
            "2026-11-06T01:00:00Z",
            "ALS가 뭐야?",
            Some("AsyncLocalStorage예요."),
        ),
        turn(
            "2026-11-06T02:00:00Z",
            "throttler 창이 왜\n1시간이야? 헷갈려",
            Some("기본값이에요."),
        ),
    ];
    let find = |quote: &str, why: &str| LearnFind {
        quote: quote.into(),
        title: String::new(),
        why: why.into(),
    };
    let read_until = chrono::DateTime::parse_from_rfc3339("2026-11-06T01:30:00Z")
        .unwrap()
        .timestamp();
    let kept = verified_finds(
        vec![
            find("ALS가 뭐야?", "question"),
            find("throttler 창이 왜 1시간이야?", "confused"),
            find("기본값이에요", "unknown"),
            find("헷갈려", "bogus"),
        ],
        &turns,
        Some(read_until),
    );
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0.quote, "throttler 창이 왜 1시간이야?");
    assert_eq!((kept[0].0.why.as_str(), kept[0].1), ("confused", 1));
    let excerpt = words_in_context(&turns, 1);
    assert!(excerpt.starts_with("에이전트(앞 답):\nAsyncLocalStorage예요."));
    assert!(excerpt.contains("사용자:\nthrottler 창이 왜\n1시간이야? 헷갈려"));
    assert!(excerpt.ends_with("에이전트(뒤 답):\n기본값이에요."));
}

#[test]
fn passed_or_waiting_words_are_not_raised_again_and_old_ones_go() {
    let proposal = |quote: &str, at: i64| Proposal {
        id: 0,
        title: "t".into(),
        quote: quote.into(),
        why: "question".into(),
        excerpt: String::new(),
        project: None,
        cwd: None,
        source: Source {
            agent: "claude".into(),
            session_id: "s1".into(),
            transcript: None,
            requests: None,
        },
        at,
    };
    let mut store = Proposals::default();
    assert!(store.add(proposal("a", 0)));
    assert!(!store.add(proposal("a", 0)));
    let id = store.proposals[0].id;
    let taken = store.take(id).unwrap();
    store.skipped.push(Skipped {
        session_id: taken.source.session_id,
        quote: taken.quote,
        at: 0,
    });
    assert!(!store.add(proposal("a", 0)));
    assert!(store.add(proposal("b", 0)));
    store.tidy(PROPOSAL_DAYS * DAY);
    assert!(store.proposals.is_empty());
    assert_eq!(store.skipped.len(), 1);
    store.tidy(SKIPPED_DAYS * DAY);
    assert!(store.skipped.is_empty());
}
