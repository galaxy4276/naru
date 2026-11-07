use super::*;

const PDF: &[u8] = b"%PDF-1.7\nrest";
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2];

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-library-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_shelved_book_keeps_its_page_and_translations_until_it_is_removed() {
    let root = scratch("shelf");
    let book = add(
        &root,
        "/Users/me/Downloads/The Mom Test.PDF",
        210,
        JPEG,
        PDF,
    )
    .unwrap();
    assert_eq!(book.title, "The Mom Test");
    assert_eq!(std::fs::read(pdf_path(&root, &book.id)).unwrap(), PDF);

    let again = add(&root, "copy.pdf", 210, JPEG, PDF).unwrap_err();
    assert!(
        again
            .to_string()
            .contains("이미 서재에 있어요: The Mom Test"),
        "{again}"
    );

    set_page(&root, &book.id, 42).unwrap();
    assert!(
        set_page(&root, &book.id, 211).is_err(),
        "past the last page"
    );
    let shelved = self::book(&root, &book.id).unwrap();
    assert_eq!(shelved.page, 42);
    assert!(shelved.opened.is_some());
    assert_eq!(
        std::fs::read_to_string(dir(&root, &book.id).join("page")).unwrap(),
        "42"
    );

    assert_eq!(translation(&root, &book.id, 42).unwrap(), None);
    save_translation(&root, &book.id, 42, "## 3장").unwrap();
    assert_eq!(
        translation(&root, &book.id, 42).unwrap().as_deref(),
        Some("## 3장")
    );

    remove(&root, &book.id).unwrap();
    assert!(!dir(&root, &book.id).exists());
    assert!(self::book(&root, &book.id).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_pdfs_with_a_jpeg_cover_are_shelved() {
    let root = scratch("kinds");
    assert!(add(&root, "a.pdf", 3, JPEG, b"<html>").is_err());
    assert!(add(&root, "a.pdf", 3, b"\x89PNG", PDF).is_err());
    assert!(add(&root, ".pdf", 3, JPEG, PDF).is_err(), "no title");
    assert!(load(&root).unwrap().books.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_unreadable_list_is_not_overwritten() {
    let root = scratch("broken");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("library.json"), "not json").unwrap();
    assert!(add(&root, "a.pdf", 3, JPEG, PDF).is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("library.json")).unwrap(),
        "not json"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn note_names_drop_what_a_file_name_cannot_hold() {
    let vault = Path::new("/vault/notes");
    let note = note_path(vault, "$100M Offers: How to / Make #1").unwrap();
    assert_eq!(
        note,
        Path::new("/vault/notes/learning/서재/$100M Offers How to Make 1.md")
    );
    assert!(note_path(vault, "///").is_err());
}

#[test]
fn the_session_task_points_at_the_book_its_page_and_its_note() {
    let root = Path::new("/lib");
    let book = Book {
        id: "abc".to_owned(),
        title: "The Mom Test".to_owned(),
        pages: 210,
        page: 42,
        added: 0,
        opened: None,
        session: None,
        sha256: String::new(),
        translatable: None,
    };
    let vault = Path::new("/vault/notes");
    let note = note_path(vault, &book.title).unwrap();
    let task = task(&book, root, vault, &note, "2026-11-06");
    assert!(task.contains("/lib/abc/book.pdf"));
    assert!(task.contains("/lib/abc/page"));
    assert!(task.contains(&note.display().to_string()));
    assert!(task.contains(
        "title: The Mom Test\nproject: learning\ntype: 학습\nreading: 읽기전\ncreated: 2026-11-06\n"
    ));
}

#[test]
fn page_texts_must_cover_every_page_and_count_what_can_be_translated() {
    let root = scratch("texts");
    let book = add(&root, "a.pdf", 3, JPEG, PDF).unwrap();
    assert!(save_texts(&root, &book.id, vec!["one".into()]).is_err());
    save_texts(
        &root,
        &book.id,
        vec!["one".into(), "  ".into(), "three".into()],
    )
    .unwrap();
    assert_eq!(self::book(&root, &book.id).unwrap().translatable, Some(2));
    assert_eq!(texts(&root, &book.id).unwrap().unwrap()[2], "three");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_question_is_one_line_that_cannot_leave_a_paste() {
    let text = ask_text(42, "first\nline \x1b[201~ end", "왜?\r").unwrap();
    assert_eq!(text, "42쪽의 \"first line [201~ end\"에 대해: 왜?");
    assert!(ask_text(1, " ", "왜?").is_err());
    assert!(ask_text(1, "글", "").is_err());
}

#[test]
fn a_translation_sees_the_end_of_the_page_before() {
    let before = "x".repeat(1000) + "the sentence runs";
    let prompt = translate_prompt("T", 2, "on here.", Some(&before));
    assert!(prompt.contains("the sentence runs\n</before>"));
    assert!(!prompt.contains(&"x".repeat(BEFORE_CHARS)));
    assert!(!translate_prompt("T", 1, "first", None).contains("<before>"));
}

#[test]
fn a_picked_pdf_is_read_from_an_absolute_path_only() {
    let dir = scratch("picked");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.pdf");
    std::fs::write(&file, PDF).unwrap();
    assert_eq!(read_picked(&file).unwrap(), PDF);
    assert!(read_picked(Path::new("a.pdf")).is_err());
    assert!(read_picked(&dir.join("missing.pdf")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
