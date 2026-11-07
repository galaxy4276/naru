use super::*;
use crate::session::Session;
use crate::state::PaneRecord;
use uuid::Uuid;

fn steps(keys: &[&str]) -> Vec<NewStep> {
    keys.iter()
        .map(|key| NewStep {
            key: (*key).into(),
            title: format!("{key} 단계"),
        })
        .collect()
}

#[test]
fn set_checks_keys_and_keeps_a_handed_steps_child() {
    assert!(set(None, "t", steps(&[]), false).is_err());
    assert!(set(None, "t", steps(&["a", "a"]), false).is_err());
    assert!(set(None, "t", steps(&["a b"]), false).is_err());
    let child = Uuid::new_v4();
    let mut plan = set(None, "결제 내보내기", steps(&["api", "ui"]), true).unwrap();
    assert!(plan.review);
    plan.steps[0].child = Some(child);
    plan.steps[0].reported = true;
    let again = set(
        Some(&plan),
        "결제 내보내기",
        steps(&["api", "ui", "docs"]),
        false,
    )
    .unwrap();
    assert_eq!(
        (again.steps[0].child, again.steps[0].reported),
        (Some(child), true)
    );
    assert!(!again.review, "set without --ask lifts the review");
    assert!(
        set(Some(&plan), "t", steps(&["ui"]), false).is_err(),
        "a handed step cannot vanish"
    );
}

#[test]
fn a_step_is_handed_once_the_review_is_over_and_while_no_child_works_it() {
    let mut plan = set(None, "t", steps(&["api"]), true).unwrap();
    assert!(
        handable(Some(&mut plan), "api", |_| true).is_err(),
        "waits for review"
    );
    plan.review = false;
    assert!(handable(Some(&mut plan), "nope", |_| true).is_err());
    let child = Uuid::new_v4();
    handable(Some(&mut plan), "api", |_| true).unwrap().child = Some(child);
    assert!(
        handable(Some(&mut plan), "api", |_| true).is_err(),
        "its child still works"
    );
    assert!(
        handable(Some(&mut plan), "api", |_| false).is_ok(),
        "its child is gone"
    );
    let mut plans = Plans::from([(Uuid::nil(), plan)]);
    assert!(letter(&mut plans, Uuid::nil(), child, true));
    assert!(
        handable(plans.get_mut(&Uuid::nil()), "api", |_| true).is_ok(),
        "it reported"
    );
}

#[test]
fn a_steps_state_comes_from_its_child_and_review_waits_for_a_resting_owner() {
    let (owner, working, asking, gone, done) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let mut plan = set(None, "t", steps(&["w", "a", "g", "d", "n"]), true).unwrap();
    for (index, child) in [working, asking, gone, done].into_iter().enumerate() {
        plan.steps[index].child = Some(child);
    }
    plan.steps[3].reported = true;
    let mut workspace = Workspace::default();
    for pane in [owner, working, asking] {
        workspace.panes.insert(pane, PaneRecord::default());
    }
    let shown = view(&plan, &workspace, &BTreeSet::from([asking]), |_| {
        Some("a2".into())
    });
    let states: Vec<StepState> = shown.steps.iter().map(|step| step.state).collect();
    use StepState::*;
    assert_eq!(states, [Working, Blocked, Closed, Reported, Waiting]);

    assert_eq!(
        review_line(&plan, &workspace, owner).as_deref(),
        Some("계획 검토 · 5단계")
    );
    let mut session = Session::new(crate::agent::Agent::Claude, None, None);
    session.status = Status::InProgress;
    workspace.panes.get_mut(&owner).unwrap().session = Some(session);
    assert_eq!(
        review_line(&plan, &workspace, owner),
        None,
        "not while it works"
    );
}
