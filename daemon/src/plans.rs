//! Plans: an agent puts up its work as steps (`naru plan set`), may ask the user to review them
//! first (`--ask`), and hands a step to a child (`naru agent spawn --parent here --plan-node
//! <key>`). A plan adds no status of its own: a plan waiting for review is laid on its agent as
//! one more thing it waits on the user for, and only while the agent rests. Where a step stands
//! is read off its child: 진행 중, 막힘 (its question), 보고 옴 (its report), 닫힘.
//!
//! Kept in `~/.naru/plans.json`, a file of the new app's own.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::{Core, Daemon};
use crate::session::Status;
use crate::state::{PaneId, Workspace};

const MAX_STEPS: usize = 30;
const MAX_TITLE: usize = 200;
const MAX_KEY: usize = 40;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Step {
    pub key: String,
    pub title: String,
    /// The child the step was handed to.
    #[serde(default)]
    pub child: Option<PaneId>,
    /// The child reported back and was not handed more since.
    #[serde(default)]
    pub reported: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Plan {
    pub title: String,
    pub steps: Vec<Step>,
    /// Waits for the user's review; no step can be handed out meanwhile.
    pub review: bool,
}

pub type Plans = BTreeMap<PaneId, Plan>;

#[derive(Deserialize, Debug, PartialEq)]
pub struct NewStep {
    pub key: String,
    pub title: String,
}

/// Where a step stands, read off its child.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Waiting,
    Working,
    Blocked,
    Reported,
    Closed,
}

impl StepState {
    fn word(self) -> &'static str {
        match self {
            StepState::Waiting => "waiting",
            StepState::Working => "working",
            StepState::Blocked => "blocked (asked you)",
            StepState::Reported => "reported",
            StepState::Closed => "closed without a report",
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StepView {
    pub key: String,
    pub title: String,
    pub state: StepState,
    pub child: Option<PaneId>,
    /// The child's agent id (a7).
    pub child_id: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlanView {
    pub title: String,
    pub review: bool,
    pub steps: Vec<StepView>,
}

fn state_of(step: &Step, workspace: &Workspace, asking: &BTreeSet<PaneId>) -> StepState {
    match step.child {
        None => StepState::Waiting,
        Some(child) if asking.contains(&child) => StepState::Blocked,
        Some(_) if step.reported => StepState::Reported,
        Some(child) if !workspace.panes.contains_key(&child) => StepState::Closed,
        Some(_) => StepState::Working,
    }
}

pub fn view(
    plan: &Plan,
    workspace: &Workspace,
    asking: &BTreeSet<PaneId>,
    id_of: impl Fn(PaneId) -> Option<String>,
) -> PlanView {
    PlanView {
        title: plan.title.clone(),
        review: plan.review,
        steps: plan
            .steps
            .iter()
            .map(|step| StepView {
                key: step.key.clone(),
                title: step.title.clone(),
                state: state_of(step, workspace, asking),
                child: step.child,
                child_id: step.child.and_then(&id_of),
            })
            .collect(),
    }
}

/// The line a plan waiting for review lays on its agent, while the agent rests and waits on the
/// user for nothing else.
pub fn review_line(plan: &Plan, workspace: &Workspace, owner: PaneId) -> Option<String> {
    if !plan.review {
        return None;
    }
    let status = workspace
        .panes
        .get(&owner)?
        .session
        .as_ref()
        .map(|session| &session.status);
    match status {
        None | Some(Status::Success | Status::Cancelled) => {
            Some(format!("계획 검토 · {}단계", plan.steps.len()))
        }
        Some(Status::InProgress | Status::Blocked { .. } | Status::Failed { .. }) => None,
    }
}

fn clean(text: &str, max: usize, what: &str) -> Result<String, String> {
    let text = crate::team::sanitize(text).replace('\n', " ");
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("empty {what}"));
    }
    if text.chars().count() > max {
        return Err(format!("{what} is over {max} characters"));
    }
    Ok(text.to_owned())
}

/// The plan `set` makes: the new steps, keeping the child and report of each key that stays.
pub fn set(
    old: Option<&Plan>,
    title: &str,
    steps: Vec<NewStep>,
    ask: bool,
) -> Result<Plan, String> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("a plan has 1 to {MAX_STEPS} steps"));
    }
    let mut keys = HashSet::new();
    let mut made = Vec::new();
    for step in steps {
        if step.key.len() > MAX_KEY
            || !step
                .key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            || step.key.is_empty()
        {
            return Err(format!(
                "step key {:?} must be letters, digits, - or _ (at most {MAX_KEY})",
                step.key
            ));
        }
        if !keys.insert(step.key.clone()) {
            return Err(format!("step key {} appears twice", step.key));
        }
        let kept = old.and_then(|old| old.steps.iter().find(|known| known.key == step.key));
        made.push(Step {
            title: clean(&step.title, MAX_TITLE, "step title")?,
            child: kept.and_then(|kept| kept.child),
            reported: kept.is_some_and(|kept| kept.reported),
            key: step.key,
        });
    }
    if let Some(old) = old
        && let Some(lost) = old
            .steps
            .iter()
            .find(|step| step.child.is_some() && !keys.contains(&step.key))
    {
        return Err(format!(
            "step {} was handed to a child; keep its key in the new plan",
            lost.key
        ));
    }
    Ok(Plan {
        title: clean(title, MAX_TITLE, "plan title")?,
        steps: made,
        review: ask,
    })
}

/// The step `key` of `plan`, if it can be handed to a new child now.
pub fn handable<'a>(
    plan: Option<&'a mut Plan>,
    key: &str,
    live: impl Fn(PaneId) -> bool,
) -> Result<&'a mut Step, String> {
    let plan = plan.ok_or("you have no plan; put one up with naru plan set")?;
    if plan.review {
        return Err(
            "your plan waits for the user's review; hand steps out once it is approved".into(),
        );
    }
    let step = plan
        .steps
        .iter_mut()
        .find(|step| step.key == key)
        .ok_or_else(|| format!("your plan has no step {key}"))?;
    if let Some(child) = step.child
        && live(child)
        && !step.reported
    {
        return Err(format!(
            "step {key} is already with a child that has not reported"
        ));
    }
    Ok(step)
}

/// Records a letter between an owner and a step's child: a report marks the step, more work
/// clears the mark. `true` when something changed.
pub fn letter(plans: &mut Plans, owner: PaneId, child: PaneId, report: bool) -> bool {
    let Some(step) = plans
        .get_mut(&owner)
        .and_then(|plan| plan.steps.iter_mut().find(|step| step.child == Some(child)))
    else {
        return false;
    };
    let changed = step.reported != report;
    step.reported = report;
    changed
}

fn plans_path(naru_dir: &Path) -> std::path::PathBuf {
    naru_dir.join("plans.json")
}

pub fn load(naru_dir: &Path, workspace: &Workspace) -> Plans {
    let mut plans: Plans = std::fs::read(plans_path(naru_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    plans.retain(|owner, _| workspace.panes.contains_key(owner));
    plans
}

fn plan_text(plan: &PlanView) -> String {
    let mut text = format!(
        "{}{}",
        plan.title,
        if plan.review {
            " (waits for the user's review)"
        } else {
            ""
        }
    );
    for step in &plan.steps {
        let child = step
            .child_id
            .as_deref()
            .map_or(String::new(), |id| format!(" · {id}"));
        text.push_str(&format!(
            "\n  {}  [{}{child}]  {}",
            step.key,
            step.state.word(),
            step.title
        ));
    }
    text
}

impl Daemon {
    pub fn save_plans(&self, core: &mut Core) {
        let workspace = &core.workspace;
        core.team
            .plans
            .retain(|owner, _| workspace.panes.contains_key(owner));
        let path = plans_path(&self.naru_dir);
        let result = serde_json::to_vec_pretty(&core.team.plans)
            .map_err(std::io::Error::other)
            .and_then(|bytes| crate::suggestions::write_private(&path, &bytes));
        if let Err(error) = result {
            eprintln!("cannot save {}: {error}", path.display());
        }
    }

    pub fn op_plan_set(
        &self,
        from: PaneId,
        title: &str,
        steps: Vec<NewStep>,
        ask: bool,
    ) -> Result<Value, String> {
        let mut core = self.core();
        let plan = set(core.team.plans.get(&from), title, steps, ask)?;
        let count = plan.steps.len();
        core.team.plans.insert(from, plan);
        self.save_plans(&mut core);
        self.send_if_team_changed(&core);
        Ok(json!({ "steps": count, "review": ask }))
    }

    /// The plan of `to` (an agent id, `parent`, or `None` for the caller).
    pub fn op_plan_show(&self, from: PaneId, to: Option<&str>) -> Result<Value, String> {
        let core = self.core();
        let owner = match to {
            None => from,
            Some("parent") => core
                .workspace
                .panes
                .get(&from)
                .and_then(|record| record.parent)
                .ok_or("this agent has no parent")?,
            Some(id) => core
                .team
                .pane_for(id)
                .ok_or_else(|| format!("no agent {id}"))?,
        };
        let plan = core.team.plans.get(&owner).ok_or("no plan")?;
        let view = core.team.plan_view(plan, &core.workspace);
        Ok(json!({ "text": plan_text(&view) }))
    }

    pub fn op_plan_approve(&self, from: PaneId) -> Result<Value, String> {
        let mut core = self.core();
        let plan = core.team.plans.get_mut(&from).ok_or("you have no plan")?;
        plan.review = false;
        self.save_plans(&mut core);
        self.send_if_team_changed(&core);
        Ok(json!({}))
    }

    pub fn op_plan_drop(&self, from: PaneId) -> Result<Value, String> {
        let mut core = self.core();
        core.team.plans.remove(&from).ok_or("you have no plan")?;
        self.save_plans(&mut core);
        self.send_if_team_changed(&core);
        Ok(json!({}))
    }

    /// The user approved `owner`'s plan on the screen: the review goes and the agent is told.
    pub fn plan_approve(&self, owner: PaneId) -> anyhow::Result<()> {
        let mut core = self.core();
        let plan = core
            .team
            .plans
            .get_mut(&owner)
            .filter(|plan| plan.review)
            .ok_or_else(|| anyhow::anyhow!("검토를 기다리는 계획이 없어요"))?;
        plan.review = false;
        self.save_plans(&mut core);
        drop(core);
        self.letter_send(
            owner,
            "계획을 승인했어요. 이대로 진행하세요 (naru plan show로 단계를 다시 볼 수 있어요).",
        )
    }
}

#[cfg(test)]
#[path = "plans_tests.rs"]
mod tests;
