use super::*;
use crate::library::add;

const PDF: &[u8] = b"%PDF-1.7\nrest";
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2];

#[test]
fn the_figure_is_the_html_document_in_the_answer() {
    let answer = "Here it is:\n<!DOCTYPE html>\n<html><body>그림</body></html>\nDone.";
    assert_eq!(
        html_of(answer),
        Some("<!DOCTYPE html>\n<html><body>그림</body></html>")
    );
    assert_eq!(html_of("<html>no end"), None);
    assert_eq!(html_of("no figure"), None);
}

#[test]
fn underlines_keep_a_little_text_on_each_side() {
    assert_eq!(tail("가나다라", 2), "다라");
    assert_eq!(clip("가나다라", 2), "가나");
    assert_eq!(tail("가", 5), "가");
}

#[test]
fn marks_and_figures_stay_with_their_book() {
    let root = std::env::temp_dir().join(format!("naru-marks-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let book = add(&root, "m.pdf", 2, JPEG, PDF).unwrap();
    edit_marks(&root, &book.id, |marks| {
        marks.marks.push(Mark {
            id: "m1".into(),
            page: 1,
            quote: "밑줄".into(),
            prefix: String::new(),
            suffix: String::new(),
            created: 0,
        });
        Ok(())
    })
    .unwrap();
    assert_eq!(load_marks(&root, &book.id).unwrap().marks.len(), 1);
    assert!(
        edit_marks(&root, "nope", |_| Ok(())).is_err(),
        "no marks for a book that is not shelved"
    );
    std::fs::write(marks_path(&root, &book.id), "broken").unwrap();
    assert!(edit_marks(&root, &book.id, |_| Ok(())).is_err());
    assert_eq!(
        std::fs::read_to_string(marks_path(&root, &book.id)).unwrap(),
        "broken"
    );
    let _ = std::fs::remove_dir_all(&root);
}
