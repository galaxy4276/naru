use super::*;

#[test]
fn keeps_front_matter_and_never_overwrites_an_outside_edit() {
    let dir = std::env::temp_dir().join(format!("naru-brain-{}", uuid::Uuid::new_v4()));
    let path = dir.join("Brain.md");
    assert_eq!(read(&path, None), Note::Offline);
    std::fs::create_dir_all(&dir).unwrap();

    let Note::Found { front, body, mtime } = read(&path, None) else {
        panic!("a missing note is created");
    };
    assert!(front.starts_with("---\ntitle: Brain\n") && front.ends_with("---\n"));
    assert_eq!(body, "\n## 지금 집중\n\n");
    assert_eq!(read(&path, Some(mtime)), Note::Unchanged { mtime });

    let Written::Saved { mtime } = write(&path, "[[링크]] 메모\n", Some(mtime)) else {
        panic!("a save over the note it saw lands");
    };
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, format!("{front}[[링크]] 메모\n"));

    std::thread::sleep(std::time::Duration::from_millis(5));
    std::fs::write(&path, "outside\n").unwrap();
    let Written::Changed { body, mtime, .. } = write(&path, "mine\n", Some(mtime)) else {
        panic!("a save over an outside edit waits for the user");
    };
    assert_eq!(body, "outside\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "outside\n");
    assert!(matches!(
        write(&path, "mine\n", mtime),
        Written::Saved { .. }
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\n");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn splits_only_a_closed_leading_front_matter() {
    assert_eq!(
        split_front_matter("---\na: 1\n---\nbody"),
        ("---\na: 1\n---\n", "body")
    );
    assert_eq!(split_front_matter("---\na: 1\n---"), ("---\na: 1\n---", ""));
    assert_eq!(split_front_matter("---\nno end"), ("", "---\nno end"));
    assert_eq!(split_front_matter("body\n---\n"), ("", "body\n---\n"));
}
