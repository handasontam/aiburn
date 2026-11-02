//! Incremental aggregation. Usage records are folded into `Report` and
//! dropped, so peak memory scales with the number of distinct groups (days /
//! months / sessions) plus whatever a loader must hold to dedup: Codex folds
//! each event as it is parsed; Claude first keeps one row per streamed
//! message, so its peak is bounded by distinct messages, not events.

use std::collections::btree_map::Entry;
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

impl Group {
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache
    }

    /// Fold another partial of the same group in: one usage record, or
    /// another thread's sum in the parallel reduce.
    fn absorb(&mut self, o: Group) {
        self.cost += o.cost;
        self.input += o.input;
        self.output += o.output;
        self.cache += o.cache;
        self.models.extend(o.models);
        // The project label follows the most recent event. Rows arrive in
        // no fixed order (Claude's dedup map, the parallel reduce), so
        // "first seen" would make the label differ from run to run.
        if !o.project.is_empty() && (self.project.is_empty() || o.last_ts > self.last_ts) {
            self.project = o.project;
        }
        if o.last_ts > self.last_ts || self.last_date.is_empty() {
            self.last_ts = o.last_ts;
            self.last_date = o.last_date;
        }
    }
}

/// What the groups can't say: per-agent totals are summed from the groups.
#[derive(Default)]
pub struct Footer {
    pub unpriced_tokens: u64,
    pub unpriced_models: BTreeSet<String>,
    /// Files that could not be opened or read to the end; their usage is
    /// missing from every number in the report.
    pub unreadable_files: usize,
}

#[derive(Default)]
pub struct Report {
    pub groups: BTreeMap<String, Group>,
    pub footer: Footer,
}

impl Report {
    /// Fold one usage record in (respecting the query filter), then let it drop.
    pub fn add(&mut self, q: &Query, r: UsageRow) {
        if !q.accepts(&r.date) {
            return;
        }
        if !r.priced {
            self.footer.unpriced_tokens += r.tokens.total();
            self.footer.unpriced_models.insert(r.model.clone());
        }
        let key = q.key(&r);
        let g = Group {
            period: q.period(&r),
            agent: r.agent,
            cost: r.cost,
            input: r.tokens.input,
            output: r.tokens.output,
            cache: r.tokens.cache_read + r.tokens.cache_write + r.tokens.cache_write_1h,
            models: BTreeSet::from([r.model]),
            project: r.project,
            last_ts: r.timestamp,
            last_date: r.date,
        };
        self.merge_group(key, g);
    }

    fn merge_group(&mut self, key: String, g: Group) {
        match self.groups.entry(key) {
            Entry::Vacant(e) => {
                e.insert(g);
            }
            Entry::Occupied(mut e) => e.get_mut().absorb(g),
        }
    }

    /// Combine two reports (used as the rayon reduce operator).
    pub fn merged(mut self, other: Report) -> Report {
        let f = &mut self.footer;
        f.unpriced_tokens += other.footer.unpriced_tokens;
        f.unpriced_models.extend(other.footer.unpriced_models);
        f.unreadable_files += other.footer.unreadable_files;
        for (k, g) in other.groups {
            self.merge_group(k, g);
        }
        self
    }

    /// (cost, tokens) summed over one agent's groups.
    pub fn agent_total(&self, agent: Agent) -> (f64, u64) {
        self.groups
            .values()
            .filter(|g| g.agent == agent)
            .fold((0.0, 0), |(c, t), g| (c + g.cost, t + g.tokens()))
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
            r.add(&q, row(Agent::Codex, d, "m", 1));
        }
        let periods: Vec<&str> = r.groups.values().map(|g| g.period.as_str()).collect();
        assert_eq!(periods, ["2026-01-02", "2026-01-03"]);
    }

    #[test]
    fn same_day_and_model_split_per_agent() {
        let q = daily(None, None);
        let mut r = Report::default();
        r.add(&q, row(Agent::Claude, "2026-01-01", "m", 1));
        r.add(&q, row(Agent::Codex, "2026-01-01", "m", 2));
        assert_eq!(r.groups.len(), 2);
        assert_eq!(r.agent_total(Agent::Claude), (1.0, 1));
        assert_eq!(r.agent_total(Agent::Codex), (1.0, 2));
    }

    #[test]
    fn latest_event_sets_project_and_last_date_regardless_of_order() {
        let q = Query {
            command: Command::Session,
            since: None,
            until: None,
        };
        let at = |timestamp, date, project: &str| UsageRow {
            timestamp,
            project: project.to_string(),
            ..row(Agent::Claude, date, "m", 1)
        };
        let early = || at(100, "2026-01-01", "old");
        let late = || at(200, "2026-01-02", "new");
        let latest = |r: &Report| {
            let g = r.groups.values().next().unwrap();
            (g.project.clone(), g.last_date.clone())
        };
        let want = ("new".to_string(), "2026-01-02".to_string());
        for order in [[early(), late()], [late(), early()]] {
            let mut r = Report::default();
            for x in order {
                r.add(&q, x);
            }
            assert_eq!(latest(&r), want);
        }
        // The same rule when the two events were folded on different threads.
        let (mut a, mut b) = (Report::default(), Report::default());
        a.add(&q, early());
        b.add(&q, late());
        assert_eq!(latest(&a.merged(b)), want);
    }

    #[test]
    fn merged_matches_single_fold() {
        // `merged` is the parallel reduce; a footer or group field it forgets
        // under-reports silently, so compare against folding serially.
        let q = daily(None, None);
        let rows = || {
            [
                row(Agent::Codex, "2026-01-01", "a", 10),
                row(Agent::Codex, "2026-01-01", "b", 20),
                row(Agent::Claude, "2026-01-02", "a", 30),
            ]
        };
        let mut one = Report::default();
        let (mut left, mut right) = (Report::default(), Report::default());
        for r in rows() {
            one.add(&q, r);
        }
        for (i, r) in rows().into_iter().enumerate() {
            if i.is_multiple_of(2) {
                left.add(&q, r)
            } else {
                right.add(&q, r)
            }
        }
        let m = left.merged(right);
        assert_eq!(m.groups.len(), one.groups.len());
        for (k, g) in &one.groups {
            assert_eq!(
                (m.groups[k].input, m.groups[k].cost),
                (g.input, g.cost),
                "{k}"
            );
        }
        assert_eq!(m.footer.unpriced_tokens, one.footer.unpriced_tokens);
    }
}
