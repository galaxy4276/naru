use super::*;

fn note(path: &str, mtime: i64) -> Note {
    Note {
        path: path.into(),
        title: path.into(),
        mtime,
        head: String::new(),
    }
}

fn item(id: u32) -> MapItem {
    MapItem {
        id,
        title: format!("m{id}"),
        hard: String::new(),
    }
}

#[test]
fn reads_only_study_notes_with_their_title() {
    let dir = std::env::temp_dir().join(format!("naru-map-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("2026-11-06/assets")).unwrap();
    std::fs::write(
        dir.join("2026-11-06/a.md"),
        "---\ntitle: \"ALS와 타이머\"\ntype: 학습\n---\n\n본문   첫\n줄",
    )
    .unwrap();
    std::fs::write(dir.join("2026-11-06/b.md"), "---\ntype: 학습\n---\nx").unwrap();
    std::fs::write(dir.join("2026-11-06/c.md"), "---\ntype: 기획\n---\nx").unwrap();
    std::fs::write(dir.join("d.md"), "no frontmatter").unwrap();
    let found = notes(&dir).unwrap();
    let titles: Vec<&str> = found.iter().map(|note| note.title.as_str()).collect();
    assert_eq!(found.len(), 2);
    assert!(titles.contains(&"ALS와 타이머") && titles.contains(&"b"));
    let als = found
        .iter()
        .find(|note| note.title == "ALS와 타이머")
        .unwrap();
    assert_eq!(als.head, "본문 첫 줄");
    assert!(notes(&dir.join("missing")).is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn groups_at_most_once_a_day_and_only_after_a_change() {
    let notes = [note("/s/a.md", 10)];
    let map = Map {
        built_on: "2026-11-05".into(),
        notes_mtime: 10,
        groups: vec![Group {
            name: "g".into(),
            marks: vec![1],
            ..Group::default()
        }],
        ..Map::default()
    };
    assert!(due(None, &notes, &[], "2026-11-06"));
    assert!(!due(Some(&map), &notes, &[1], "2026-11-06"));
    assert!(due(Some(&map), &notes, &[1, 2], "2026-11-06"));
    assert!(due(Some(&map), &[note("/s/a.md", 11)], &[1], "2026-11-06"));
    assert!(!due(Some(&map), &[note("/s/a.md", 11)], &[1], "2026-11-05"));
    assert!(!due(None, &[], &[], "2026-11-06"));
}

#[test]
fn keeps_only_known_items_placed_once() {
    let notes = [note("/s/a.md", 1), note("/s/b.md", 2)];
    let answer = r#"여기 있어요 {"groups":[
        {"name":"Node 런타임","items":["n1","m3","n9","x"]},
        {"name":"DB","items":["n1","n2","p4"]},
        {"name":"빈","items":["n7"]}],
      "links":[{"a":"n1","b":"m3","why":"메모리 한도"},{"a":"n2","b":"m8","why":"없는 것"},
               {"a":"n2","b":"n2","why":"자기 자신"}]}"#;
    let (groups, links) = parse(answer, &notes, &[item(3)], &[item(4)]).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].notes, ["/s/a.md"]);
    assert_eq!(groups[0].marks, [3]);
    assert_eq!(groups[1].notes, ["/s/b.md"]);
    assert_eq!(groups[1].proposals, [4]);
    assert_eq!(links.len(), 1);
    assert_eq!(
        (links[0].a.as_str(), links[0].b.as_str()),
        ("n:/s/a.md", "m:3")
    );
    assert!(parse("묶지 못했어요", &notes, &[], &[]).is_none());
    let map = Map {
        groups,
        ..Map::default()
    };
    assert_eq!(map.related(3, &notes), [PathBuf::from("/s/a.md")]);
    assert!(map.related(9, &notes).is_empty());
}
