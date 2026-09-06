//! Incremental aggregation. Usage records are folded into `Report` and
//! dropped, so peak memory scales with the number of distinct groups (days /
//! months / sessions) plus whatever a loader must hold to dedup: Codex folds
//! each event as it is parsed; Claude first keeps one row per streamed
//! message, so its peak is bounded by distinct messages, not events.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Agent, UsageRow};

#[derive(Clone, Copy, PartialEq)]
pub enum Command {
    Daily,
    Monthly,
    Session,
}

/// The report request: how to group, and which dates to include. Agent
/// selection happens upstream by not scanning the other agent's files.
pub struct Query {
    pub command: Command,
    pub since: Option<String>,
    pub until: Option<String>,
}

impl Query {
    /// Inclusive on both ends, comparing local YYYY-MM-DD strings.
    fn accepts(&self, date: &str) -> bool {
        if let Some(s) = &self.since {
            if date < s.as_str() {
                return false;
            }
        }
        if let Some(u) = &self.until {
            if date > u.as_str() {
                return false;
            }
        }
        true
    }

    /// Unique key per group; the map's order is the display order. Every
    /// group is scoped to a single agent so that Claude and Codex get
    /// separate rows/totals. Daily/monthly rows are further split per raw
    /// model; a session stays one row, listing its models in the Models
    /// column.
    fn key(&self, r: &UsageRow) -> String {
        match self.command {
            Command::Monthly => format!("{}|{}|{}", r.month, r.agent.as_str(), r.model),
            Command::Session => format!("{}|{}", r.agent.as_str(), r.session_id),
            Command::Daily => format!("{}|{}|{}", r.date, r.agent.as_str(), r.model),
        }
    }

    /// Human-facing period label for the group (date / month / session id).
    fn period(&self, r: &UsageRow) -> String {
        match self.command {
            Command::Monthly => r.month.clone(),
            Command::Session => r.session_id.clone(),
            Command::Daily => r.date.clone(),
        }
    }
}

#[derive(Default)]
pub struct Group {
    pub period: String,
    pub agent: Agent,
    pub cost: f64,
    pub input: u64,
    pub output: u64,
    pub cache: u64,
    pub models: BTreeSet<String>,
    pub project: String,
    pub last_ts: i64,
    /// Local YYYY-MM-DD of the most recent event, so the session table's
    /// "Last active" agrees with the daily view and `--since`/`--until`.
    pub last_date: String,
}

#[derive(Default)]
pub struct Footer {
    pub claude_cost: f64,
    pub codex_cost: f64,
    pub claude_tokens: u64,
    pub codex_tokens: u64,
    pub unpriced_tokens: u64,
    pub unpriced_models: BTreeSet<String>,
    /// Files that could not be opened or read to the end; their usage is
    /// missing from every number above.
    pub unreadable_files: usize,
}

#[derive(Default)]
pub struct Report {
    pub groups: BTreeMap<String, Group>,
    pub footer: Footer,
}

impl Report {
    /// Fold one usage record in (respecting the query filter), then let it drop.
    pub fn add(&mut self, q: &Query, r: &UsageRow) {
        if !q.accepts(&r.date) {
            return;
        }
        let tok = r.tokens.total();
        match r.agent {
            Agent::Claude => {
                self.footer.claude_cost += r.cost;
                self.footer.claude_tokens += tok;
            }
            Agent::Codex => {
                self.footer.codex_cost += r.cost;
                self.footer.codex_tokens += tok;
            }
        }
        if !r.priced {
            self.footer.unpriced_tokens += tok;
            self.footer.unpriced_models.insert(r.model.clone());
        }

        let g = self.groups.entry(q.key(r)).or_insert_with(|| Group {
            period: q.period(r),
            agent: r.agent,
            ..Default::default()
        });
        g.cost += r.cost;
        g.input += r.tokens.input;
        g.output += r.tokens.output;
        g.cache += r.tokens.cache_read + r.tokens.cache_write + r.tokens.cache_write_1h;
        if !g.models.contains(&r.model) {
            g.models.insert(r.model.clone());
        }
        // The project label follows the most recent event. Rows arrive in
        // no fixed order (Claude's dedup map, the parallel reduce), so
        // "first seen" would make the label differ from run to run.
        if !r.project.is_empty() && (g.project.is_empty() || r.timestamp > g.last_ts) {
            g.project = r.project.clone();
        }
        if r.timestamp > g.last_ts || g.last_date.is_empty() {
            g.last_ts = r.timestamp;
            g.last_date = r.date.clone();
        }
    }

    /// Combine two reports (used as the rayon reduce operator).
    pub fn merged(mut self, other: Report) -> Report {
        let f = &mut self.footer;
        f.claude_cost += other.footer.claude_cost;
        f.codex_cost += other.footer.codex_cost;
        f.claude_tokens += other.footer.claude_tokens;
        f.codex_tokens += other.footer.codex_tokens;
        f.unpriced_tokens += other.footer.unpriced_tokens;
        f.unpriced_models.extend(other.footer.unpriced_models);
        f.unreadable_files += other.footer.unreadable_files;

        for (k, og) in other.groups {
            let g = self.groups.entry(k).or_insert_with(|| Group {
                period: og.period.clone(),
                agent: og.agent,
                ..Default::default()
            });
            g.cost += og.cost;
            g.input += og.input;
            g.output += og.output;
            g.cache += og.cache;
            g.models.extend(og.models);
            if !og.project.is_empty() && (g.project.is_empty() || og.last_ts > g.last_ts) {
                g.project = og.project;
            }
            if og.last_ts > g.last_ts || g.last_date.is_empty() {
                g.last_ts = og.last_ts;
                g.last_date = og.last_date;
            }
        }
        self
    }

    /// Groups in display order: the map's key order (period, agent, model),
    /// or most recent first for sessions.
    pub fn sorted_groups(&self, command: Command) -> Vec<&Group> {
        let mut v: Vec<&Group> = self.groups.values().collect();
        if command == Command::Session {
            v.sort_by_key(|g| std::cmp::Reverse(g.last_ts));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tokens;

    fn row(agent: Agent, date: &str, model: &str, input: u64) -> UsageRow {
        UsageRow {
            agent,
            timestamp: 0,
            date: date.to_string(),
            month: date[..7].to_string(),
            session_id: "s".to_string(),
            project: String::new(),
            model: model.to_string(),
            tokens: Tokens {
                input,
                ..Default::default()
            },
            cost: 1.0,
            priced: true,
        }
    }

    fn daily(since: Option<&str>, until: Option<&str>) -> Query {
        Query {
            command: Command::Daily,
            since: since.map(str::to_string),
            until: until.map(str::to_string),
        }
    }

    #[test]
    fn date_bounds_are_inclusive() {
        // Making either bound exclusive silently drops a whole day from
        // every report.
        let q = daily(Some("2026-01-02"), Some("2026-01-03"));
        let mut r = Report::default();
        for d in ["2026-01-01", "2026-01-02", "2026-01-03", "2026-01-04"] {
            r.add(&q, &row(Agent::Codex, d, "m", 1));
        }
        let periods: Vec<&str> = r.groups.values().map(|g| g.period.as_str()).collect();
        assert_eq!(periods, ["2026-01-02", "2026-01-03"]);
    }

    #[test]
    fn same_day_and_model_split_per_agent() {
        let q = daily(None, None);
        let mut r = Report::default();
        r.add(&q, &row(Agent::Claude, "2026-01-01", "m", 1));
        r.add(&q, &row(Agent::Codex, "2026-01-01", "m", 1));
        assert_eq!(r.groups.len(), 2);
        assert_eq!((r.footer.claude_cost, r.footer.codex_cost), (1.0, 1.0));
    }

    #[test]
    fn latest_event_sets_project_and_last_date_regardless_of_order() {
        let q = Query {
            command: Command::Session,
            since: None,
            until: None,
        };
        let mut early = row(Agent::Claude, "2026-01-01", "m", 1);
        early.timestamp = 100;
        early.project = "old".to_string();
        let mut late = row(Agent::Claude, "2026-01-02", "m", 1);
        late.timestamp = 200;
        late.project = "new".to_string();
        let latest = |r: &Report| {
            let g = r.groups.values().next().unwrap();
            (g.project.clone(), g.last_date.clone())
        };
        let want = ("new".to_string(), "2026-01-02".to_string());
        for order in [[&early, &late], [&late, &early]] {
            let mut r = Report::default();
            for x in order {
                r.add(&q, x);
            }
            assert_eq!(latest(&r), want);
        }
        // The same rule when the two events were folded on different threads.
        let (mut a, mut b) = (Report::default(), Report::default());
        a.add(&q, &early);
        b.add(&q, &late);
        assert_eq!(latest(&a.merged(b)), want);
    }

    #[test]
    fn merged_matches_single_fold() {
        // `merged` is the parallel reduce; a footer or group field it forgets
        // under-reports silently, so compare against folding serially.
        let q = daily(None, None);
        let rows = [
            row(Agent::Codex, "2026-01-01", "a", 10),
            row(Agent::Codex, "2026-01-01", "b", 20),
            row(Agent::Claude, "2026-01-02", "a", 30),
        ];
        let mut one = Report::default();
        let (mut left, mut right) = (Report::default(), Report::default());
        for (i, r) in rows.iter().enumerate() {
            one.add(&q, r);
            if i.is_multiple_of(2) {
                left.add(&q, r)
            } else {
                right.add(&q, r)
            }
        }
        let m = left.merged(right);
        assert_eq!(m.groups.len(), one.groups.len());
        for (k, g) in &one.groups {
            assert_eq!(m.groups[k].input, g.input, "{k}");
        }
        assert_eq!(m.footer.claude_tokens, one.footer.claude_tokens);
        assert_eq!(m.footer.codex_tokens, one.footer.codex_tokens);
        assert_eq!(m.footer.codex_cost, one.footer.codex_cost);
    }
}
