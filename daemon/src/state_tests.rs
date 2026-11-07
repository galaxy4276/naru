use super::*;

#[test]
fn splits_nest_and_collapse_back() {
    let mut workspace = Workspace::default();
    let (tab, first) = workspace.add_tab(None, Some("/tmp".into()));
    let right = workspace.split(first, Direction::Right, true).unwrap();
    let third = workspace.split(right, Direction::Right, true).unwrap();
    let below = workspace.split(third, Direction::Down, true).unwrap();
    assert_eq!(workspace.panes[&below].cwd.as_deref(), Some("/tmp"));
    let layout = &workspace.tabs[0].layout;
    assert_eq!(layout.panes(), vec![first, right, third, below]);
    let Layout::Split { row, children, .. } = layout else {
        panic!("split")
    };
    assert!(row);
    assert_eq!(children.len(), 3);

    workspace.remove_pane(below);
    workspace.remove_pane(right);
    assert_eq!(workspace.tabs[0].layout.panes(), vec![first, third]);
    assert_eq!(workspace.tabs[0].focused, first);
    workspace.remove_pane(third);
    assert_eq!(workspace.tabs[0].layout, Layout::Pane { pane: first });
    workspace.remove_pane(first);
    assert!(workspace.tabs.is_empty());
    assert!(workspace.panes.is_empty());
    assert_eq!(workspace.active_tab, None);
    let _ = tab;
}

#[test]
fn new_tabs_join_the_end_of_their_group() {
    let mut workspace = Workspace::default();
    let api = workspace.add_group("api".into(), Some("/p/api".into()));
    let app = workspace.add_group("app".into(), Some("/p/app".into()));
    let (a1, _) = workspace.add_tab(Some(api), None);
    let (b1, _) = workspace.add_tab(Some(app), None);
    let (a2, _) = workspace.add_tab(Some(api), None);
    let order: Vec<Id> = workspace.tabs.iter().map(|tab| tab.id).collect();
    assert_eq!(order, vec![a1, a2, b1]);
    assert_eq!(workspace.active_tab, Some(a2));
    assert_eq!(workspace.group_for("/p/api/src"), Some(api));
    assert_eq!(workspace.group_for("/p/apiary"), None);
    let tmp = workspace.add_group("tmp".into(), Some("/tmp/naru".into()));
    assert_eq!(workspace.group_for("/private/tmp/naru/src"), Some(tmp));
    assert_eq!(workspace.group_for("/privateer/tmp/naru"), None);
}

#[test]
fn saves_and_loads_whole() {
    let dir = std::env::temp_dir().join(format!("naru-state-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");
    assert_eq!(Workspace::load(&path).unwrap(), Workspace::default());
    let mut workspace = Workspace::default();
    let (_, pane) = workspace.add_tab(None, Some("/tmp".into()));
    let mut session = crate::session::Session::new(crate::agent::Agent::Claude, None, None);
    session.session_id = Some("abc".into());
    workspace.panes.get_mut(&pane).unwrap().session = Some(session);
    workspace.save(&path).unwrap();
    let mut loaded = Workspace::load(&path).unwrap();
    loaded.repair();
    assert_eq!(loaded, workspace);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn repair_drops_strays_and_fixes_focus() {
    let mut workspace = Workspace::default();
    let (_, pane) = workspace.add_tab(None, None);
    workspace
        .panes
        .insert(Uuid::new_v4(), PaneRecord::default());
    workspace.tabs[0].focused = Uuid::new_v4();
    workspace.tabs[0].group = Some(99);
    workspace.active_tab = Some(42);
    workspace.repair();
    assert_eq!(workspace.panes.len(), 1);
    assert_eq!(workspace.tabs[0].focused, pane);
    assert_eq!(workspace.tabs[0].group, None);
    assert_eq!(workspace.active_tab, Some(workspace.tabs[0].id));
}

#[test]
fn renames_a_group_but_not_to_nothing() {
    let mut workspace = Workspace::default();
    let api = workspace.add_group("api".into(), None);
    workspace.rename_group(api, "  backend ").unwrap();
    assert_eq!(workspace.groups[0].name, "backend");
    assert!(workspace.rename_group(api, "   ").is_err());
    assert!(workspace.rename_group(api + 99, "x").is_err());
    assert_eq!(workspace.groups[0].name, "backend");
}

#[test]
fn a_restart_resumes_the_card_in_view_and_its_project_first() {
    let mut workspace = Workspace::default();
    let api = workspace.add_group("api".into(), None);
    let app = workspace.add_group("app".into(), None);
    let (_, other) = workspace.add_tab(Some(app), None);
    let (_, sibling) = workspace.add_tab(Some(api), None);
    let (_, left) = workspace.add_tab(Some(api), None);
    let right = workspace.split(left, Direction::Right, true).unwrap();
    // add_tab and split focus what they open; the user is looking at the right pane.
    assert_eq!(workspace.focused(), Some(right));
    assert_eq!(workspace.resume_order(), vec![right, left, sibling, other]);
}
