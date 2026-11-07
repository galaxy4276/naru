use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-bg-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_saved_image_reads_back_and_only_naru_files_are_removed() {
    let root = scratch("own");
    let dir = root.join("session-backgrounds");
    let saved = save(&dir, PNG).unwrap();
    assert_eq!(saved.extension().unwrap(), "png");
    assert_eq!(read(&saved).unwrap(), ("image/png", PNG.to_vec()));

    let users = root.join("fork.png");
    std::fs::write(&users, PNG).unwrap();
    remove_own(&dir, &users);
    assert!(users.exists(), "a file outside Naru's folder stays");
    remove_own(&dir, &saved);
    assert!(!saved.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_png_and_jpeg_are_kept() {
    let dir = scratch("kind").join("session-backgrounds");
    assert!(save(&dir, b"GIF89a").is_err());
    assert!(
        save(&dir, &[0xff, 0xd8, 0xff, 0xe0])
            .unwrap()
            .to_string_lossy()
            .ends_with(".jpg")
    );
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}
