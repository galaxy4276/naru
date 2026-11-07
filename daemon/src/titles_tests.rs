use super::*;

#[test]
fn reads_title_summary_and_candidates_from_model_output() {
    let output = "\n\"웹소켓 재연결 리플레이 중복 고치기\"\n요약: 재연결 때 리플레이가 두 번 찍히는 원인을 찾는 중\n";
    assert_eq!(
        clean_title(output).as_deref(),
        Some("웹소켓 재연결 리플레이 중복 고치기")
    );
    assert_eq!(
        clean_summary(output).as_deref(),
        Some("재연결 때 리플레이가 두 번 찍히는 원인을 찾는 중")
    );
    assert_eq!(clean_summary("제목만"), None);
    assert_eq!(
        clean_title("제목: 소켓 고치기").as_deref(),
        Some("소켓 고치기")
    );
    let long = format!("t\n{}", "가".repeat(70));
    assert_eq!(
        clean_summary(&long).unwrap().chars().count(),
        MAX_SUMMARY_CHARS
    );
    assert_eq!(
        parse_candidates("1. 재연결 버그\n- \"리플레이 중복\"\n2) 재연결 버그\n소켓 정리\n넷째"),
        ["재연결 버그", "리플레이 중복", "소켓 정리"]
    );
}

#[test]
fn digests_the_last_requests_without_injected_ones() {
    let jsonl = [
        r#"{"type":"user","message":{"content":"<system-reminder>x</system-reminder>"}}"#,
        r#"{"type":"user","message":{"content":"리플레이가 두 번 찍혀"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"reset을 큐로 보냈어요"}]}}"#,
    ]
    .join("\n");
    assert_eq!(
        digest(Agent::Claude, &jsonl).as_deref(),
        Some("User: 리플레이가 두 번 찍혀\n\nAssistant: reset을 큐로 보냈어요")
    );
    assert_eq!(digest(Agent::Claude, ""), None);
}
