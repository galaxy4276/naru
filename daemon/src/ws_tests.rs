use super::*;

#[test]
fn allows_only_the_app_and_local_dev_pages() {
    for origin in [
        None,
        Some("tauri://localhost"),
        Some("http://localhost:5173"),
        Some("http://127.0.0.1:5173"),
    ] {
        assert!(origin_allowed(origin), "{origin:?}");
    }
    for origin in [
        "https://example.com",
        "http://localhost.evil.com:80",
        "http://localhost:",
        "null",
    ] {
        assert!(!origin_allowed(Some(origin)), "{origin}");
    }
}

#[test]
fn lane_files_come_as_markdown_text_or_image() {
    let dir = std::env::temp_dir().join(format!("naru-view-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let kind = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        view_body(1, &path, false).map(|body| serde_json::to_value(body).unwrap()["kind"].clone())
    };
    assert_eq!(kind("a.md", b"# hi").unwrap(), "markdown");
    assert_eq!(kind("a.log", b"plain").unwrap(), "text");
    assert_eq!(kind("a.PNG", b"\x89PNG").unwrap(), "image");
    assert!(kind("a.bin", b"\xff\xfe\x00").is_err());
    assert_eq!(kind("a.html", b"<p>hi</p>").unwrap(), "text");
    let page = dir.join("p.html");
    std::fs::write(&page, "<p>hi</p>").unwrap();
    let body = serde_json::to_value(view_body(1, &page, true).unwrap()).unwrap();
    assert_eq!(body["kind"], "html");
    assert_eq!(body["text"], "<p>hi</p>");
    std::fs::remove_dir_all(dir).unwrap();
}
