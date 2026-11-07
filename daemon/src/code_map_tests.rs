use super::*;

const DIFF: &str = "diff --git a/web/src/App.tsx b/web/src/App.tsx
index 1..2 100644
--- a/web/src/App.tsx
+++ b/web/src/App.tsx
@@ -3,0 +4,2 @@ import x
+import { A } from './ui/A';
+import { B } from '@/ui/B';
@@ -10,2 +12 @@ fn
-old one
-old two
+new
@@ -20 +20,0 @@
-gone
diff --git a/old.rs b/old.rs
deleted file mode 100644
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn x() {}
";

#[test]
fn diff_lines_land_on_the_file_as_it_is_now() {
    let files = parse_diff(DIFF);
    let app = &files["web/src/App.tsx"];
    assert_eq!(app.marks, vec![4, 5, 12]);
    assert_eq!((app.added, app.removed), (3, 3));
    assert_eq!(
        app.gone,
        vec![
            Gone {
                after: 11,
                lines: vec!["old one".into(), "old two".into()]
            },
            Gone {
                after: 20,
                lines: vec!["gone".into()]
            },
        ]
    );
    assert_eq!(files["old.rs"].status, Status::Deleted);
}

fn file(path: &str, lines: &[&str]) -> MapFile {
    MapFile {
        path: path.into(),
        lines: lines.iter().map(|l| l.to_string()).collect(),
        ..MapFile::default()
    }
}

#[test]
fn imports_join_changed_files() {
    let files = vec![
        file("daemon/src/devtools.rs", &[]),
        file("daemon/src/main.rs", &["mod devtools;"]),
        file(
            "daemon/src/ws.rs",
            &["let r = crate::devtools::servers(&s);"],
        ),
        file(
            "web/src/App.tsx",
            &[
                "import { A } from './ui/A';",
                "import type { X } from '@/connection';",
            ],
        ),
        file("web/src/connection.ts", &[]),
        file(
            "web/src/ui/A.tsx",
            &[
                "import { y } from '../connection';",
                "import z from 'react';",
            ],
        ),
    ];
    assert_eq!(edges(&files), vec![(1, 0), (2, 0), (3, 4), (3, 5), (5, 4)]);
}

#[test]
fn reads_a_real_checkout() {
    let dir = std::env::temp_dir().join(format!("naru-code-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let run = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    run(&["init", "-q"]);
    std::fs::write(dir.join("a.ts"), "one\ntwo\n").unwrap();
    run(&["add", "."]);
    run(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "x",
    ]);
    std::fs::write(dir.join("a.ts"), "one\n2\nthree\n").unwrap();
    std::fs::write(dir.join("b.ts"), "import { a } from './a';\n").unwrap();
    let map = read(&dir).unwrap();
    let paths: Vec<_> = map.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["a.ts", "b.ts"]);
    assert_eq!(map.files[0].marks, vec![2, 3]);
    assert_eq!(map.files[1].status, Status::New);
    assert_eq!(map.edges, vec![(1, 0)]);
    std::fs::remove_dir_all(&dir).unwrap();
}
