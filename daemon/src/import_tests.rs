use super::*;

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
CREATE TABLE windows (id INTEGER PRIMARY KEY, active_tab_index INTEGER NOT NULL);
CREATE TABLE tab_groups (id INTEGER PRIMARY KEY, window_id INTEGER NOT NULL, name TEXT, color TEXT,
  collapsed BOOLEAN NOT NULL, pinned BOOLEAN NOT NULL DEFAULT FALSE, project_path TEXT);
CREATE TABLE tabs (id INTEGER PRIMARY KEY, window_id INTEGER NOT NULL, custom_title TEXT, color TEXT,
  tab_group_id INTEGER, pinned BOOLEAN NOT NULL DEFAULT FALSE, background_image TEXT);
CREATE TABLE pane_nodes (id INTEGER PRIMARY KEY, tab_id INTEGER NOT NULL, parent_pane_node_id INTEGER,
  flex FLOAT, is_leaf BOOLEAN NOT NULL);
CREATE TABLE pane_branches (id INTEGER PRIMARY KEY, pane_node_id INTEGER NOT NULL UNIQUE, horizontal BOOLEAN NOT NULL);
CREATE TABLE pane_leaves (pane_node_id INTEGER NOT NULL UNIQUE, kind TEXT NOT NULL,
  is_focused BOOLEAN NOT NULL DEFAULT FALSE, custom_vertical_tabs_title TEXT);
CREATE TABLE terminal_panes (id INTEGER PRIMARY KEY, uuid BLOB NOT NULL UNIQUE, cwd TEXT, cli_agent TEXT,
  cli_agent_session_id TEXT, cli_agent_launcher TEXT, cli_agent_lifecycle TEXT);
CREATE TABLE naru_agent_parents (child BLOB NOT NULL PRIMARY KEY, parent BLOB NOT NULL);
";

const ROWS: &str = r#"
INSERT INTO windows VALUES (1, 1);
INSERT INTO tab_groups VALUES (1, 1, 'naru', NULL, 0, 0, '/x/naru');
INSERT INTO tab_groups VALUES (2, 1, NULL, NULL, 0, 0, NULL);
INSERT INTO tabs VALUES (1, 1, 'notes ', NULL, NULL, 0, NULL);
INSERT INTO tabs VALUES (2, 1, NULL, NULL, 1, 1, NULL);
INSERT INTO tabs VALUES (3, 1, 'code only', NULL, 1, 0, NULL);
INSERT INTO pane_nodes VALUES (1, 1, NULL, NULL, 1);
INSERT INTO pane_leaves VALUES (1, 'terminal', 1, NULL);
INSERT INTO terminal_panes VALUES (1, X'11111111111141118111111111111111', '/x', NULL, NULL, NULL, NULL);
INSERT INTO pane_nodes VALUES (2, 2, NULL, NULL, 0);
INSERT INTO pane_branches VALUES (1, 2, 1);
INSERT INTO pane_nodes VALUES (3, 2, 2, 0.6, 1);
INSERT INTO pane_nodes VALUES (4, 2, 2, 0.2, 1);
INSERT INTO pane_nodes VALUES (5, 2, 2, 0.4, 1);
INSERT INTO pane_leaves VALUES (3, 'terminal', 0, '내가 친 제목');
INSERT INTO pane_leaves VALUES (4, 'code', 0, NULL);
INSERT INTO pane_leaves VALUES (5, 'terminal', 1, '자동 제목');
INSERT INTO terminal_panes VALUES (3, X'22222222222242228222222222222222', '/x/naru', 'Claude',
  '0f554f23-ee48-4a48-85d3-73aa72d1dc18', 'cl',
  '["Active",{"activity_summary":"리플레이 고치는 중","launch_command":null,"shows_done":true}]');
INSERT INTO terminal_panes VALUES (5, X'33333333333343338333333333333333', '/x/naru', 'Codex',
  '$(rm -rf ~)', 'codex', '["Active",{"activity_summary":null}]');
INSERT INTO naru_agent_parents VALUES (X'33333333333343338333333333333333', X'22222222222242228222222222222222');
INSERT INTO pane_nodes VALUES (6, 3, NULL, NULL, 1);
INSERT INTO pane_leaves VALUES (6, 'code', 1, NULL);
PRAGMA wal_checkpoint(TRUNCATE);
"#;

#[test]
fn imports_tabs_splits_titles_sessions_and_teams_from_the_fork() {
    let dir = std::env::temp_dir().join(format!("naru-import-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("warp.sqlite");
    let _ = std::fs::remove_file(&db);
    let made = Command::new(SQLITE)
        .arg(&db)
        .arg(format!("{SCHEMA}{ROWS}"))
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    // As the fork leaves it after quitting: WAL mode with no -shm, which read-only opens refuse.
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    std::fs::write(dir.join("typed-titles.json"), r#"["내가 친 제목"]"#).unwrap();

    let ws = import(&db, &dir).unwrap();
    let [shell, claude, codex] = [
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        "33333333-3333-4333-8333-333333333333",
    ]
    .map(|id| PaneId::parse_str(id).unwrap());

    assert_eq!(
        ws.groups.len(),
        1,
        "a group with no name and no folder is dropped"
    );
    assert_eq!(ws.tabs.len(), 2, "a tab with no terminal pane is dropped");
    assert_eq!(ws.tabs[0].title.as_deref(), Some("notes"));
    assert_eq!(ws.tabs[0].group, None);
    let split = &ws.tabs[1];
    assert_eq!(split.group, Some(ws.groups[0].id));
    assert!(split.pinned);
    assert_eq!(
        split.layout,
        Layout::Split {
            row: true,
            children: vec![Layout::Pane { pane: claude }, Layout::Pane { pane: codex }],
            flex: vec![0.6, 0.4]
        }
    );
    assert_eq!(split.focused, codex);
    assert_eq!(ws.active_tab, Some(split.id), "window's active tab index 1");

    assert_eq!(ws.panes[&shell].session, None);
    let record = &ws.panes[&claude];
    assert_eq!(record.title.as_deref(), Some("내가 친 제목"));
    assert_eq!(record.auto_title, None);
    assert_eq!(record.summary.as_deref(), Some("리플레이 고치는 중"));
    let session = record.session.as_ref().unwrap();
    assert_eq!(
        session.resume_command().as_deref(),
        Some("cl --resume 0f554f23-ee48-4a48-85d3-73aa72d1dc18")
    );
    assert!(
        session.shows_done(),
        "a turn the fork showed as done stays done"
    );

    let kid = &ws.panes[&codex];
    assert_eq!(kid.auto_title.as_deref(), Some("자동 제목"));
    assert_eq!(kid.parent, Some(claude));
    let kid_session = kid.session.as_ref().unwrap();
    assert_eq!(
        kid_session.session_id, None,
        "an unsafe session id never reaches a command line"
    );
    assert_eq!(kid_session.resume_command(), None);
    assert!(!kid_session.shows_done());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thumbnails_come_by_group_from_the_forks_folder_hash() {
    let home = std::env::temp_dir().join(format!("naru-thumbs-test-{}", std::process::id()));
    let dir = home.join(".warp-oss/project-thumbnails");
    std::fs::create_dir_all(&dir).unwrap();
    let name: String = Sha256::digest(b"/x/naru")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    std::fs::write(dir.join(format!("{name}.png")), b"png").unwrap();
    let group = |id, path: Option<&str>| Group {
        id,
        name: format!("g{id}"),
        path: path.map(str::to_owned),
        color: None,
        collapsed: false,
        pinned: false,
    };
    let groups = [
        group(1, Some("/x/naru")),
        group(2, Some("/x/other")),
        group(3, None),
    ];

    let found = thumbnails(&home, &groups);

    assert_eq!(found.len(), 1, "only groups with a saved thumbnail");
    assert_eq!(BASE64.decode(&found[&1]).unwrap(), b"png");
    let _ = std::fs::remove_dir_all(&home);
}
