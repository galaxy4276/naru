use super::*;
use crate::library::{add, save_texts, set_page};

const PDF: &[u8] = b"%PDF-1.7\nrest";
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2];

#[test]
fn the_queue_starts_at_the_open_page_and_skips_what_is_done_or_empty() {
    let root = std::env::temp_dir().join(format!("naru-queue-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let book = add(&root, "q.pdf", 5, JPEG, PDF).unwrap();
    assert!(next(&root).is_none(), "no text yet");
    let texts = ["one", "two", "", "four", "five"]
        .map(String::from)
        .to_vec();
    save_texts(&root, &book.id, texts).unwrap();
    set_page(&root, &book.id, 3).unwrap();
    super::super::save_translation(&root, &book.id, 4, "넷").unwrap();

    let first = next(&root).unwrap();
    assert_eq!((first.page, first.text.as_str()), (5, "five"));
    assert_eq!(first.before.as_deref(), Some("four"));
    let second = next(&root).unwrap();
    assert_eq!(
        second.page, 1,
        "wraps to the start; 3 is empty and 4 is done"
    );
    assert_eq!(second.before, None);
    assert_eq!(next(&root).unwrap().page, 2);
    assert!(next(&root).is_none(), "the rest are claimed");

    let mut busy = TRANSLATING.lock().unwrap();
    for page in [1, 2, 5] {
        busy.remove(&(book.id.clone(), page));
    }
    let _ = std::fs::remove_dir_all(&root);
}
