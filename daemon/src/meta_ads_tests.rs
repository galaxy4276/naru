use std::cell::RefCell;

use super::*;

type Posts = Vec<(String, Vec<(String, String)>)>;

/// Meta, answering GETs from a list and recording POSTs.
#[derive(Default)]
struct Fake {
    gets: Vec<(&'static str, Value)>,
    posts: RefCell<Posts>,
}

impl Graph for Fake {
    fn get(&self, path: &str, _query: &[(&str, &str)]) -> anyhow::Result<Value> {
        self.gets
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix))
            .map(|(_, value)| value.clone())
            .ok_or_else(|| anyhow!("unexpected GET {path}"))
    }

    fn post(&self, path: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
        let form = form
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.posts.borrow_mut().push((path.to_owned(), form));
        Ok(json!({ "success": true }))
    }
}

fn campaigns() -> Value {
    serde_json::from_str(include_str!("testdata/meta_ads/campaigns.json")).unwrap()
}

#[test]
fn campaign_rows_take_their_budget_metrics_and_objective_result() {
    let graph = Fake {
        gets: vec![("act_1/campaigns", campaigns())],
        ..Default::default()
    };
    let rows = rows(&graph, "act_1", Level::Campaign, None, "last_7d").unwrap();
    assert_eq!(rows.len(), 3);

    // CBO campaign: the budget sits on the campaign; result is app installs with their cost.
    let app = &rows[0];
    assert_eq!(
        app.budget,
        Some(Budget {
            kind: BudgetKind::Daily,
            minor: 30000
        })
    );
    let metrics = app.metrics.as_ref().unwrap();
    assert_eq!(
        (metrics.spend, metrics.impressions, metrics.clicks),
        (Some(182000.0), Some(51234), Some(812))
    );
    assert_eq!(
        app.outcome,
        Some(Outcome {
            label: "앱 설치",
            count: 97.0,
            cost: Some(1876.29)
        })
    );

    // ABO campaign with no delivery in the period: no budget here, no metrics, not zeros.
    let paused = &rows[1];
    assert_eq!(paused.budget, None);
    assert_eq!(paused.metrics, None);
    assert_eq!(paused.outcome, None);
    assert_eq!(paused.status, "PAUSED");

    // An objective with no single action and a lifetime budget.
    let awareness = &rows[2];
    assert_eq!(
        awareness.budget,
        Some(Budget {
            kind: BudgetKind::Lifetime,
            minor: 1_000_000
        })
    );
    assert!(awareness.metrics.is_some());
    assert_eq!(awareness.outcome, None);
}

#[test]
fn an_ad_set_reads_its_campaign_objective_and_follows_paging() {
    let first = json!({
        "data": [{ "id": "11", "name": "a", "status": "ACTIVE", "effective_status": "ACTIVE",
                   "campaign": { "objective": "OUTCOME_LEADS", "id": "9" }, "daily_budget": "5000",
                   "insights": { "data": [{ "spend": "100", "actions": [{ "action_type": "lead", "value": "4" }] }] } }],
        "paging": { "next": "https://graph.facebook.com/v26.0/9/adsets?after=x" }
    });
    let second = json!({ "data": [{ "id": "12", "name": "b", "status": "PAUSED" }] });
    let graph = Fake {
        gets: vec![("9/adsets", first), ("https://graph.facebook.com/", second)],
        ..Default::default()
    };
    let rows = rows(&graph, "act_1", Level::Adset, Some("9"), "today").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].outcome,
        Some(Outcome {
            label: "리드",
            count: 4.0,
            cost: None
        })
    );
    assert_eq!(
        rows[0].budget,
        Some(Budget {
            kind: BudgetKind::Daily,
            minor: 5000
        })
    );
}

#[test]
fn requests_outside_the_contract_are_refused() {
    let graph = Fake::default();
    assert!(rows(&graph, "act_1", Level::Campaign, None, "last_90d").is_err());
    assert!(rows(&graph, "1", Level::Campaign, None, "today").is_err());
    assert!(rows(&graph, "act_1", Level::Ad, Some("../me"), "today").is_err());
    assert!(rows(&graph, "act_1", Level::Adset, None, "today").is_err());
}

#[test]
fn unknown_currencies_get_no_offset() {
    assert_eq!(offset("KRW"), Some(1));
    assert_eq!(offset("USD"), Some(100));
    assert_eq!(offset("XYZ"), None);
}

fn current(budget: &str, status: &str) -> Fake {
    Fake {
        gets: vec![(
            "42",
            json!({ "id": "42", "name": "앱 설치", "status": status, "daily_budget": budget, "lifetime_budget": "0" }),
        )],
        ..Default::default()
    }
}

#[test]
fn a_budget_change_goes_through_when_meta_still_holds_what_the_screen_saw() {
    let dir = tempdir();
    let log = dir.join("log.jsonl");
    let graph = current("30000", "ACTIVE");
    act(
        &graph,
        &log,
        "act_1",
        "42",
        &Act::Budget {
            field: BudgetKind::Daily,
            from: 30000,
            to: 50000,
        },
    )
    .unwrap();
    assert_eq!(
        *graph.posts.borrow(),
        [(
            "42".to_owned(),
            vec![("daily_budget".to_owned(), "50000".to_owned())]
        )]
    );
    let line: Value = serde_json::from_str(std::fs::read_to_string(&log).unwrap().trim()).unwrap();
    assert_eq!(
        (
            line["field"].as_str(),
            line["from"].as_i64(),
            line["to"].as_str()
        ),
        (Some("daily_budget"), Some(30000), Some("50000"))
    );
}

#[test]
fn a_change_is_refused_when_meta_holds_another_value() {
    let dir = tempdir();
    let log = dir.join("log.jsonl");
    let graph = current("40000", "ACTIVE");
    let error = act(
        &graph,
        &log,
        "act_1",
        "42",
        &Act::Budget {
            field: BudgetKind::Daily,
            from: 30000,
            to: 50000,
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), STALE);
    let error = act(
        &graph,
        &log,
        "act_1",
        "42",
        &Act::Status {
            from: "PAUSED".into(),
            to: "ACTIVE".into(),
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), STALE);
    assert!(graph.posts.borrow().is_empty());
    assert!(!log.exists());
}

#[test]
fn only_pause_resume_and_positive_budgets_are_sent() {
    let dir = tempdir();
    let log = dir.join("log.jsonl");
    let graph = current("30000", "ACTIVE");
    assert!(
        act(
            &graph,
            &log,
            "act_1",
            "42",
            &Act::Status {
                from: "ACTIVE".into(),
                to: "DELETED".into()
            }
        )
        .is_err()
    );
    assert!(
        act(
            &graph,
            &log,
            "act_1",
            "42",
            &Act::Budget {
                field: BudgetKind::Daily,
                from: 30000,
                to: 0
            }
        )
        .is_err()
    );
    assert!(graph.posts.borrow().is_empty());
    act(
        &graph,
        &log,
        "act_1",
        "42",
        &Act::Status {
            from: "ACTIVE".into(),
            to: "PAUSED".into(),
        },
    )
    .unwrap();
    assert_eq!(
        graph.posts.borrow()[0].1,
        [("status".to_owned(), "PAUSED".to_owned())]
    );
}

#[test]
fn the_token_shows_by_its_last_four_characters() {
    assert_eq!(tail("EAABwzLixnjYBO1234abcd"), "abcd");
    assert_eq!(tail("ab"), "ab");
}

#[test]
fn only_meta_paging_links_carry_the_token() {
    assert!(Http::url("https://evil.example/x").is_err());
    assert_eq!(
        Http::url("act_1/campaigns").unwrap(),
        format!("{GRAPH}/act_1/campaigns")
    );
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naru-meta-ads-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
