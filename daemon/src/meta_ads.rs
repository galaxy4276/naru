//! What the Meta 광고 panel shows and changes: an ad account's campaigns, ad sets and ads with their
//! insights, read through the Marketing API with a system user token, and pausing, resuming or
//! rebudgeting one of them. A change goes through only when Meta still holds the value the screen
//! showed, and every change that went through is appended to `meta-ads-log.jsonl`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const GRAPH: &str = "https://graph.facebook.com/v26.0";
pub const PRESETS: [&str; 4] = ["today", "yesterday", "last_7d", "last_30d"];
/// Rows one level may list; a level past this is cut and says so.
const MAX_ROWS: usize = 500;
const INSIGHTS: &str = "spend,impressions,clicks,ctr,cpc,actions,cost_per_action_type";

/// `~/.naru/meta-ads.json`, readable by the user alone.
#[derive(Default, Serialize, Deserialize)]
struct Config {
    token: Option<String>,
    /// The ad account the panel showed last.
    account: Option<String>,
}

fn config_path(naru_dir: &Path) -> PathBuf {
    naru_dir.join("meta-ads.json")
}

fn load(naru_dir: &Path) -> anyhow::Result<Config> {
    let path = config_path(naru_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{}을 읽지 못했어요", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error).with_context(|| format!("{}을 읽지 못했어요", path.display())),
    }
}

fn save(naru_dir: &Path, config: &Config) -> anyhow::Result<()> {
    let path = config_path(naru_dir);
    crate::suggestions::write_private(&path, &serde_json::to_vec_pretty(config)?)
        .with_context(|| format!("{}에 쓰지 못했어요", path.display()))
}

/// The Marketing API, behind a seam so tests can stand in for Meta.
pub trait Graph {
    /// `path` is relative to the API version, or a `paging.next` URL Meta handed back.
    fn get(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<Value>;
    fn post(&self, path: &str, form: &[(&str, &str)]) -> anyhow::Result<Value>;
}

pub struct Http {
    token: String,
}

impl Http {
    fn url(path: &str) -> anyhow::Result<String> {
        if path.starts_with("https://") {
            // The token goes along, so only Meta's own paging links are followed.
            if !path.starts_with("https://graph.facebook.com/") {
                bail!("Meta가 아닌 주소예요: {path}");
            }
            return Ok(path.to_owned());
        }
        Ok(format!("{GRAPH}/{path}"))
    }

    fn reply(
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> anyhow::Result<Value> {
        let mut response = result.map_err(|error| anyhow!("Meta에 닿지 못했어요: {error}"))?;
        let status = response.status();
        let text = response
            .body_mut()
            .read_to_string()
            .context("Meta 응답을 읽지 못했어요")?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("Meta 응답이 JSON이 아니에요 ({status})"))?;
        if let Some(message) = value["error"]["error_user_msg"]
            .as_str()
            .or(value["error"]["message"].as_str())
        {
            bail!("Meta: {message}");
        }
        if !status.is_success() {
            bail!("Meta가 {status}로 답했어요");
        }
        Ok(value)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into()
}

impl Graph for Http {
    fn get(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<Value> {
        let request = agent()
            .get(Self::url(path)?)
            .header("Authorization", format!("Bearer {}", self.token))
            .query_pairs(query.iter().copied());
        Self::reply(request.call())
    }

    fn post(&self, path: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
        let request = agent()
            .post(Self::url(path)?)
            .header("Authorization", format!("Bearer {}", self.token));
        Self::reply(request.send_form(form.iter().copied()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Campaign,
    Adset,
    Ad,
}

#[derive(Debug, Serialize)]
pub struct AdAccount {
    pub id: String,
    pub name: String,
    pub currency: String,
    /// Minor units per major unit; `None` for a currency this panel does not know, whose budgets
    /// it shows but does not let the user change.
    pub offset: Option<u32>,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    Daily,
    Lifetime,
}

impl BudgetKind {
    fn field(self) -> &'static str {
        match self {
            Self::Daily => "daily_budget",
            Self::Lifetime => "lifetime_budget",
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Budget {
    pub kind: BudgetKind,
    /// In the account currency's minor unit, as Meta keeps it.
    pub minor: i64,
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Metrics {
    pub spend: Option<f64>,
    pub impressions: Option<u64>,
    pub clicks: Option<u64>,
    pub ctr: Option<f64>,
    pub cpc: Option<f64>,
}

/// The one result the campaign's objective counts, as the Ads Manager 결과 column does.
#[derive(Debug, PartialEq, Serialize)]
pub struct Outcome {
    pub label: &'static str,
    pub count: f64,
    pub cost: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub level: Level,
    /// What the user set: ACTIVE, PAUSED, …
    pub status: String,
    /// What Meta delivers, after parents and review: ACTIVE, CAMPAIGN_PAUSED, DISAPPROVED, …
    pub effective_status: String,
    pub budget: Option<Budget>,
    /// `None` when Meta has no insights for the period (nothing delivered), never zeros.
    pub metrics: Option<Metrics>,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Serialize)]
pub struct View {
    /// The saved token's last four characters; `None` when no token is saved.
    pub token_tail: Option<String>,
    /// Listed with the top level only; expanding a row does not list them again.
    pub accounts: Option<Vec<AdAccount>>,
    pub account: Option<String>,
    pub preset: String,
    pub level: Level,
    pub parent: Option<String>,
    pub rows: Vec<Row>,
    pub error: Option<String>,
}

/// Meta's currency offsets (Marketing API, Currencies). Only currencies checked against that
/// table are here; a missing one leaves budgets read-only rather than guessing a factor.
fn offset(currency: &str) -> Option<u32> {
    match currency {
        "KRW" | "JPY" | "TWD" | "VND" | "CLP" | "COP" | "CRC" | "HUF" | "ISK" | "IDR" | "PYG" => {
            Some(1)
        }
        "USD" | "EUR" | "GBP" | "CAD" | "AUD" | "NZD" | "SGD" | "HKD" | "CHF" | "SEK" | "NOK"
        | "DKK" | "MXN" | "BRL" | "INR" | "THB" | "PHP" | "MYR" => Some(100),
        _ => None,
    }
}

/// The action an objective's result counts. `None` for objectives with no single action
/// (awareness counts reach, which `actions` does not carry).
// ponytail: by campaign objective; Ads Manager follows each ad set's optimization goal, which
// differs for custom conversions. Read `optimization_goal` + `promoted_object` if that matters.
fn result_action(objective: &str) -> Option<(&'static str, &'static str)> {
    Some(match objective {
        "OUTCOME_APP_PROMOTION" | "APP_INSTALLS" => ("mobile_app_install", "앱 설치"),
        "OUTCOME_LEADS" | "LEAD_GENERATION" => ("lead", "리드"),
        "OUTCOME_SALES" | "CONVERSIONS" => ("purchase", "구매"),
        "OUTCOME_TRAFFIC" | "LINK_CLICKS" => ("link_click", "링크 클릭"),
        "OUTCOME_ENGAGEMENT" | "POST_ENGAGEMENT" => ("post_engagement", "참여"),
        _ => return None,
    })
}

/// Meta sends numbers as strings.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn action_value(list: &Value, action: &str) -> Option<f64> {
    list.as_array()?
        .iter()
        .find(|entry| entry["action_type"] == action)
        .and_then(|entry| number(&entry["value"]))
}

fn parse_row(level: Level, item: &Value) -> anyhow::Result<Row> {
    let text = |key: &str| item[key].as_str().map(str::to_owned);
    let id = text("id").ok_or_else(|| anyhow!("Meta 응답에 id가 없어요: {item}"))?;
    let budget = [BudgetKind::Daily, BudgetKind::Lifetime]
        .into_iter()
        .find_map(|kind| {
            let minor = number(&item[kind.field()])? as i64;
            // Meta sends "0" for the field a CBO/ABO split does not use.
            (minor > 0).then_some(Budget { kind, minor })
        });
    let insights = &item["insights"]["data"][0];
    let metrics = insights.is_object().then(|| Metrics {
        spend: number(&insights["spend"]),
        impressions: number(&insights["impressions"]).map(|n| n as u64),
        clicks: number(&insights["clicks"]).map(|n| n as u64),
        ctr: number(&insights["ctr"]),
        cpc: number(&insights["cpc"]),
    });
    let objective = item["objective"]
        .as_str()
        .or(item["campaign"]["objective"].as_str())
        .unwrap_or("");
    let outcome = result_action(objective).and_then(|(action, label)| {
        Some(Outcome {
            label,
            count: action_value(&insights["actions"], action)?,
            cost: action_value(&insights["cost_per_action_type"], action),
        })
    });
    Ok(Row {
        name: text("name").unwrap_or_else(|| id.clone()),
        id,
        level,
        status: text("status").ok_or_else(|| anyhow!("Meta 응답에 status가 없어요"))?,
        effective_status: text("effective_status").unwrap_or_default(),
        budget,
        metrics,
        outcome,
    })
}

fn is_account(id: &str) -> bool {
    id.strip_prefix("act_").is_some_and(is_object_id)
}

fn is_object_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// Every page of a list, up to `MAX_ROWS`.
fn list(
    graph: &dyn Graph,
    path: &str,
    query: &[(&str, &str)],
) -> anyhow::Result<(Vec<Value>, bool)> {
    let mut items = Vec::new();
    let mut page = graph.get(path, query)?;
    loop {
        items.extend(page["data"].as_array().cloned().unwrap_or_default());
        if items.len() >= MAX_ROWS {
            items.truncate(MAX_ROWS);
            return Ok((items, true));
        }
        match page["paging"]["next"].as_str() {
            Some(next) => page = graph.get(next, &[])?,
            None => return Ok((items, false)),
        }
    }
}

pub fn accounts(graph: &dyn Graph) -> anyhow::Result<Vec<AdAccount>> {
    let (items, _) = list(
        graph,
        "me/adaccounts",
        &[
            ("fields", "id,name,currency,account_status"),
            ("limit", "100"),
        ],
    )?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let currency = item["currency"].as_str()?.to_owned();
            Some(AdAccount {
                id: item["id"].as_str()?.to_owned(),
                name: item["name"].as_str().unwrap_or("").to_owned(),
                offset: offset(&currency),
                currency,
                active: item["account_status"].as_i64() == Some(1),
            })
        })
        .collect())
}

/// One level of the tree: the account's campaigns, a campaign's ad sets, or an ad set's ads.
pub fn rows(
    graph: &dyn Graph,
    account: &str,
    level: Level,
    parent: Option<&str>,
    preset: &str,
) -> anyhow::Result<Vec<Row>> {
    if !PRESETS.contains(&preset) {
        bail!("모르는 기간이에요: {preset}");
    }
    let path = match (level, parent) {
        (Level::Campaign, _) if is_account(account) => format!("{account}/campaigns"),
        (Level::Adset, Some(parent)) if is_object_id(parent) => format!("{parent}/adsets"),
        (Level::Ad, Some(parent)) if is_object_id(parent) => format!("{parent}/ads"),
        _ => bail!("잘못된 요청이에요: {account} {level:?} {parent:?}"),
    };
    let objective = if level == Level::Campaign {
        "objective"
    } else {
        "campaign{objective}"
    };
    let fields = format!(
        "id,name,status,effective_status,{objective},daily_budget,lifetime_budget,insights.date_preset({preset}){{{INSIGHTS}}}"
    );
    let fields = if level == Level::Ad {
        fields.replace(",daily_budget,lifetime_budget", "")
    } else {
        fields
    };
    let (items, cut) = list(graph, &path, &[("fields", &fields), ("limit", "100")])?;
    if cut {
        bail!("{MAX_ROWS}개가 넘어 보여 주지 않았어요. 광고 관리자에서 봐 주세요");
    }
    items.iter().map(|item| parse_row(level, item)).collect()
}

/// A change the user confirmed, with the value the screen showed when they did.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Act {
    Status {
        from: String,
        to: String,
    },
    Budget {
        field: BudgetKind,
        from: i64,
        to: i64,
    },
}

const STALE: &str = "다른 곳에서 바뀌었어요. 새 값을 확인해 주세요";

/// Re-reads `id`, and changes it only when it still holds `from`. Logs what went through.
pub fn act(
    graph: &dyn Graph,
    log: &Path,
    account: &str,
    id: &str,
    act: &Act,
) -> anyhow::Result<()> {
    if !is_account(account) || !is_object_id(id) {
        bail!("잘못된 요청이에요: {account} {id}");
    }
    let now = graph.get(
        id,
        &[("fields", "name,status,daily_budget,lifetime_budget")],
    )?;
    let (field, from, to) = match act {
        Act::Status { from, to } => {
            if !matches!(to.as_str(), "ACTIVE" | "PAUSED") {
                bail!("켜기·끄기만 할 수 있어요: {to}");
            }
            if now["status"].as_str() != Some(from) {
                bail!(STALE);
            }
            ("status", json!(from), to.clone())
        }
        Act::Budget { field, from, to } => {
            if *to <= 0 {
                bail!("예산은 0보다 커야 해요");
            }
            if number(&now[field.field()]).map(|n| n as i64) != Some(*from) {
                bail!(STALE);
            }
            (field.field(), json!(from), to.to_string())
        }
    };
    let done = graph.post(id, &[(field, &to)])?;
    if done["success"] != true {
        bail!("Meta가 바꿨다고 답하지 않았어요: {done}");
    }
    let line = json!({
        "at": chrono::Local::now().to_rfc3339(),
        "account": account,
        "id": id,
        "name": now["name"],
        "field": field,
        "from": from,
        "to": to,
    });
    append(log, &line)
        .map_err(|error| anyhow!("Meta에는 반영됐어요. 기록을 남기지 못했어요: {error}"))
}

fn append(path: &Path, line: &Value) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    writeln!(options.open(path)?, "{line}")
}

fn tail(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// What the panel asks for. `account` switches the shown account and is remembered.
pub fn view(
    naru_dir: &Path,
    account: Option<String>,
    level: Level,
    parent: Option<String>,
    preset: String,
) -> View {
    let mut view = View {
        token_tail: None,
        accounts: None,
        account: None,
        preset,
        level,
        parent,
        rows: Vec::new(),
        error: None,
    };
    if let Err(error) = fill(naru_dir, account, &mut view) {
        view.error = Some(format!("{error:#}"));
    }
    view
}

fn fill(naru_dir: &Path, account: Option<String>, view: &mut View) -> anyhow::Result<()> {
    let mut config = load(naru_dir)?;
    let Some(token) = config.token.clone() else {
        return Ok(());
    };
    view.token_tail = Some(tail(&token));
    let graph = Http { token };
    let account = match account.or(config.account.clone()) {
        Some(account) => account,
        None if view.level == Level::Campaign => {
            let listed = accounts(&graph)?;
            let first = listed.first().map(|account| account.id.clone());
            view.accounts = Some(listed);
            first.ok_or_else(|| anyhow!("이 토큰으로 볼 수 있는 광고 계정이 없어요"))?
        }
        None => bail!("광고 계정을 먼저 골라 주세요"),
    };
    if view.level == Level::Campaign && view.accounts.is_none() {
        view.accounts = Some(accounts(&graph)?);
    }
    if config.account.as_deref() != Some(&account) {
        config.account = Some(account.clone());
        save(naru_dir, &config)?;
    }
    view.account = Some(account.clone());
    view.rows = rows(
        &graph,
        &account,
        view.level,
        view.parent.as_deref(),
        &view.preset,
    )?;
    Ok(())
}

/// Saves a token after Meta accepts it, or forgets the saved one.
pub fn set_token(naru_dir: &Path, token: Option<String>) -> anyhow::Result<()> {
    let config = match token.map(|token| token.trim().to_owned()) {
        None => Config::default(),
        Some(token) => {
            if token.is_empty() || token.chars().any(char::is_whitespace) {
                bail!("토큰 모양이 아니에요");
            }
            Http {
                token: token.clone(),
            }
            .get("me", &[("fields", "id,name")])?;
            Config {
                token: Some(token),
                account: None,
            }
        }
    };
    save(naru_dir, &config)
}

pub fn change(naru_dir: &Path, account: &str, id: &str, act: &Act) -> anyhow::Result<()> {
    let token = load(naru_dir)?
        .token
        .ok_or_else(|| anyhow!("설정에서 토큰을 먼저 넣어 주세요"))?;
    self::act(
        &Http { token },
        &naru_dir.join("meta-ads-log.jsonl"),
        account,
        id,
        act,
    )
}

#[cfg(test)]
#[path = "meta_ads_tests.rs"]
mod tests;
