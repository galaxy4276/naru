use super::*;
use crate::state::Group;

fn group(id: u64, name: &str, path: &str) -> Group {
    Group {
        id,
        name: name.into(),
        path: Some(path.into()),
        color: None,
        collapsed: false,
        pinned: false,
    }
}

fn day() -> NaiveDate {
    Local::now().date_naive() - Duration::days(DAYS as i64 - 1)
}

fn stamp(days_after_first: i64) -> String {
    let at = Local
        .from_local_datetime(
            &(day() + Duration::days(days_after_first))
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .unwrap();
    at.to_rfc3339()
}

#[test]
fn a_claude_message_counts_once_and_is_priced_by_its_cache_ttl() {
    let at = stamp(1);
    let usage = r#"{"input_tokens":1000000,"output_tokens":100000,"cache_read_input_tokens":1000000,"cache_creation_input_tokens":2000000,"cache_creation":{"ephemeral_5m_input_tokens":1000000,"ephemeral_1h_input_tokens":1000000}}"#;
    let text = format!(
        "{{\"type\":\"user\",\"cwd\":\"/p/app/src\",\"message\":{{\"content\":\"고쳐 줘\"}}}}\n\
         {{\"timestamp\":\"{at}\",\"message\":{{\"id\":\"m1\",\"model\":\"claude-opus-5-5\",\"usage\":{usage}}}}}\n\
         {{\"timestamp\":\"{at}\",\"message\":{{\"id\":\"m1\",\"model\":\"claude-opus-5-5\",\"usage\":{usage}}}}}\n\
         {{\"timestamp\":\"{at}\",\"message\":{{\"id\":\"m2\",\"model\":\"claude-mystery\",\"usage\":{{\"input_tokens\":5}}}}}}\n"
    );
    let read = read_claude(&text, "s1".into());
    assert_eq!(read.cwd.as_deref(), Some("/p/app/src"));
    assert_eq!(read.first.as_deref(), Some("고쳐 줘"));
    assert_eq!(read.calls.len(), 2);
    // 1M in $4 + 0.1M out $2 + 1M read $0.20 + 1M 5m write $5 + 1M 1h write $8
    let cost = read.calls[0].cost.unwrap();
    assert!((cost - 19.2).abs() < 1e-9, "{cost}");
    assert_eq!(read.calls[1].cost, None);
}

#[test]
fn haiku_switches_to_its_long_prompt_price() {
    assert_eq!(
        claude_price("claude-haiku-5-5", 100_000).unwrap().input,
        0.10
    );
    assert_eq!(
        claude_price("claude-haiku-5-5", 100_001).unwrap().input,
        0.50
    );
    assert_eq!(claude_price("sonnet", 10), None);
}

#[test]
fn codex_calls_are_the_growth_of_the_running_total() {
    let (a, b) = (stamp(0), stamp(2));
    let total = |input: u64, cached: u64, output: u64| {
        format!(
            "{{\"input_tokens\":{input},\"cached_input_tokens\":{cached},\"output_tokens\":{output},\"reasoning_output_tokens\":7}}"
        )
    };
    let text = format!(
        "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"c1\",\"cwd\":\"/p/app\"}}}}\n\
         {{\"type\":\"turn_context\",\"payload\":{{\"model\":\"gpt-6.1-sol\"}}}}\n\
         {{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"테스트 추가\"}}}}\n\
         {{\"timestamp\":\"{a}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{},\"last_usage_includes_reasoning\":true}}}}}}\n\
         {{\"timestamp\":\"{a}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{},\"last_usage_includes_reasoning\":true}}}}}}\n\
         {{\"timestamp\":\"{b}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{},\"last_usage_includes_reasoning\":true}}}}}}\n",
        total(1_000_000, 500_000, 100_000),
        total(1_000_000, 500_000, 100_000),
        total(3_000_000, 1_500_000, 100_000),
    );
    let read = read_codex(&text);
    assert_eq!(
        (read.session.as_str(), read.first.as_deref()),
        ("c1", Some("테스트 추가"))
    );
    assert_eq!(read.calls.len(), 2, "a repeated total is no call");
    // 0.5M in $1 + 0.5M cached $0.05 + 0.1M out $1
    assert!((read.calls[0].cost.unwrap() - 2.05).abs() < 1e-9);
    // 1M in $2 + 1M cached $0.10
    assert!((read.calls[1].cost.unwrap() - 2.10).abs() < 1e-9);
}

#[test]
fn calls_add_up_by_project_day_and_session() {
    let mut workspace = Workspace::default();
    workspace.groups = vec![group(1, "app", "/p/app"), group(2, "api", "/p/app/api")];
    let call = |at: i64, cost: Option<f64>| Call {
        at,
        model: "claude-opus-5-5".into(),
        tokens: 10,
        cost,
    };
    let noon = |offset: i64| {
        Local
            .from_local_datetime(
                &(day() + Duration::days(offset))
                    .and_hms_opt(12, 0, 0)
                    .unwrap(),
            )
            .unwrap()
            .timestamp()
    };
    let read = |session: &str, cwd: &str, calls: Vec<Call>| {
        Arc::new(Read {
            session: session.into(),
            agent: "claude",
            cwd: Some(cwd.into()),
            first: Some(format!("{session} 작업")),
            calls,
        })
    };
    let reads = vec![
        read(
            "a",
            "/p/app",
            vec![
                call(noon(0), Some(1.0)),
                call(noon(6), Some(2.0)),
                call(noon(-1), Some(50.0)),
            ],
        ),
        read(
            "b",
            "/p/app/api/src",
            vec![call(noon(3), Some(4.0)), call(noon(3), None)],
        ),
        read("c", "/elsewhere", vec![call(noon(1), Some(0.5))]),
        read("d", "/p/application", vec![call(noon(2), Some(0.25))]),
    ];
    let costs = total(&reads, &workspace, day());
    assert_eq!(costs.days.len(), DAYS);
    let names: Vec<&str> = costs.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["api", "app", "프로젝트 밖"]);
    let api = &costs.projects[0];
    assert_eq!((api.total, api.unpriced, api.days[3]), (4.0, 1, 4.0));
    assert_eq!(
        api.models[0].cost, None,
        "a model with an unpriced call has no total"
    );
    let app = &costs.projects[1];
    assert_eq!(
        (app.total, app.days[0], app.days[6]),
        (3.0, 1.0, 2.0),
        "the day before the week is out"
    );
    assert_eq!(app.top[0].title, "a 작업");
    let outside = &costs.projects[2];
    assert_eq!(
        (outside.total, outside.sessions, outside.path.as_deref()),
        (0.75, 2, None)
    );
}
