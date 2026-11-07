use super::*;

#[test]
fn a_printed_path_names_a_file_from_the_pane_folder_or_home() {
    let root = std::env::temp_dir().join(format!("naru-terminal-path-{}", std::process::id()));
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(home.join("notes")).unwrap();
    std::fs::write(project.join("src/main.rs"), "").unwrap();
    std::fs::write(home.join("notes/a.md"), "").unwrap();
    let cwd = Some(project.as_path());

    let main = project.join("src/main.rs");
    assert_eq!(terminal_path("src/main.rs", cwd, &home), Some(main.clone()));
    assert_eq!(
        terminal_path("src/main.rs:12:5", cwd, &home),
        Some(main.clone())
    );
    assert_eq!(
        terminal_path(&format!("{}:3", main.display()), None, &home),
        Some(main)
    );
    assert_eq!(
        terminal_path("~/notes/a.md", None, &home),
        Some(home.join("notes/a.md"))
    );
    assert_eq!(
        terminal_path("src/main.rs", None, &home),
        None,
        "relative needs the folder"
    );
    assert_eq!(
        terminal_path("src", cwd, &home),
        None,
        "folders are not opened"
    );
    assert_eq!(terminal_path("src/gone.rs", cwd, &home), None);
    std::fs::remove_dir_all(root).unwrap();
}
