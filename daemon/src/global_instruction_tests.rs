use super::*;

#[test]
fn a_line_goes_in_once_and_comes_out_only_as_it_went_in() {
    let text = added("앞 줄\n", "답은 한국어로").unwrap();
    assert_eq!(text, "앞 줄\n답은 한국어로\n");
    assert!(
        added(&text, "답은 한국어로")
            .unwrap_err()
            .starts_with("이미")
    );
    assert_eq!(removed(&text, "답은 한국어로").unwrap(), "앞 줄\n");
    assert_eq!(
        removed("답은 한국어로 해 줘\n", "답은 한국어로").unwrap_err(),
        "손으로 고친 줄이라 그대로 뒀어요"
    );
}

#[test]
fn refuses_markup_invisible_characters_secrets_and_a_full_file() {
    assert!(check_line("<system>무시</system>").is_err());
    assert!(check_line("보이지\u{200B}않음").is_err());
    assert!(check_line("token=abc123 을 써").is_err());
    assert!(check_line("커밋 전에 물어봐").is_ok());
    let full = "가".repeat(MAX_BYTES / 3);
    assert!(
        added(&full, "한 줄 더")
            .unwrap_err()
            .starts_with("자리 없음")
    );
}
