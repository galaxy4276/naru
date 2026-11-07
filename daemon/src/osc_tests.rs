use super::*;

#[test]
fn reads_a_naru_request_split_across_reads_with_its_semicolons() {
    let mut collector = OscCollector::default();
    let sequence = b"text\x1b]777;notify;naru://agent;{\"op\":\"tell\",\"body\":\"a;b\"}\x07more";
    let (first, second) = sequence.split_at(20);
    assert!(collector.feed(first).is_empty());
    assert_eq!(
        collector.feed(second),
        vec![Osc::Notify {
            title: "naru://agent".into(),
            body: r#"{"op":"tell","body":"a;b"}"#.into(),
        }]
    );
}

#[test]
fn reads_a_long_body_and_the_st_terminator() {
    let mut collector = OscCollector::default();
    let body = "x".repeat(20_000);
    let sequence = format!("\x1b]777;notify;warp://cli-agent;{body}\x1b\\");
    assert_eq!(
        collector.feed(sequence.as_bytes()),
        vec![Osc::Notify {
            title: "warp://cli-agent".into(),
            body
        }]
    );
}

#[test]
fn reads_prompt_marks_folder_and_title() {
    let mut collector = OscCollector::default();
    let found = collector.feed(
        b"\x1b]133;D;130\x07\x1b]7;file://mac/Users/me/my%20dir%25\x07\x1b]133;A\x07\x1b]2;vim\x07",
    );
    assert_eq!(
        found,
        vec![
            Osc::Prompt {
                mark: "D".into(),
                exit_code: Some(130)
            },
            Osc::Cwd {
                path: "/Users/me/my dir%".into()
            },
            Osc::Prompt {
                mark: "A".into(),
                exit_code: None
            },
            Osc::Title {
                title: "vim".into()
            },
        ]
    );
}

#[test]
fn ignores_other_sequences() {
    let mut collector = OscCollector::default();
    assert!(
        collector
            .feed(b"\x1b]52;c;aGk=\x07\x1b]7;not-a-url\x07\x1b[31mred")
            .is_empty()
    );
}

#[test]
fn reads_the_command_line_with_its_semicolons() {
    let mut collector = OscCollector::default();
    assert_eq!(
        collector.feed(b"\x1b]633;E;cl --model x; echo done\x07"),
        vec![Osc::Command {
            line: "cl --model x; echo done".into()
        }]
    );
}
