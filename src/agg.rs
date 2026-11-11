//! Incremental aggregation. Usage records are folded into `Report` and
//! dropped, so peak memory scales with the number of distinct groups (days /
//! months / sessions) plus whatever a loader must hold to dedup: Codex folds
//! each event as it is parsed; Claude first keeps one row per streamed
//! message, so its peak is bounded by distinct messages, not events.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Agent, UsageRow};
use crate::pricing::cost_for;
use crate::time::Day;

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
    pub since: Option<Day>,
    pub until: Option<Day>,
}

impl Query {
    fn accepts(&self, date: &Day) -> bool {
        self.since.as_ref().is_none_or(|s| date >= s)
            && self.until.as_ref().is_none_or(|u| date <= u)
    }

    fn period(&self, r: &UsageRow) -> String {
        match self.command {
            Command::Monthly => r.date.month().to_string(),
            Command::Session => r.session_id.clone(),
            Command::Daily => r.date.as_str().to_string(),
        }
    }
}

/// (period, agent, model); a session keeps its models in one group. The map's
/// order is the display order.
type Key = (String, Agent, String);

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
    pub last_date: Day,
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
        if o.last_ts > self.last_ts {
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
    pub groups: BTreeMap<Key, Group>,
    pub footer: Footer,
}

impl Report {
    /// Price one usage record and fold it in (respecting the query filter),
    /// then let it drop.
    pub fn add(&mut self, q: &Query, r: UsageRow) {
        if !q.accepts(&r.date) {
            return;
        }
        let cost = cost_for(&r.model, &r.tokens, r.at_ms);
        if cost.is_none() {
            self.footer.unpriced_tokens += r.tokens.total();
            self.footer.unpriced_models.insert(r.model.clone());
        }
        let period = q.period(&r);
        let model = match q.command {
            Command::Session => String::new(),
            Command::Daily | Command::Monthly => r.model.clone(),
        };
        let key = (period.clone(), r.agent, model);
        let g = Group {
            period,
            agent: r.agent,
            cost: cost.unwrap_or(0.0),
            input: r.tokens.input,
            output: r.tokens.output,
            cache: r.tokens.cache_read + r.tokens.cache_write_5m + r.tokens.cache_write_1h,
            models: BTreeSet::from([r.model]),
            project: r.project,
            last_ts: r.at_ms,
            last_date: r.date,
        };
        self.merge_group(key, g);
    }

    fn merge_group(&mut self, key: Key, g: Group) {
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
            at_ms: 0,
            date: Day::parse(date).unwrap(),
            session_id: "s".to_string(),
            project: String::new(),
            model: model.to_string(),
            tokens: Tokens {
                input,
                ..Default::default()
            },
        }
    }

    fn daily(since: Option<&str>, until: Option<&str>) -> Query {
        Query {
            command: Command::Daily,
            since: since.and_then(Day::parse),
            until: until.and_then(Day::parse),
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
        assert_eq!(r.agent_total(Agent::Claude).1, 1);
        assert_eq!(r.agent_total(Agent::Codex).1, 2);
    }

    #[test]
    fn latest_event_sets_project_and_last_date_regardless_of_order() {
        let q = Query {
            command: Command::Session,
            since: None,
            until: None,
        };
        let at = |at_ms, date, project: &str| UsageRow {
            at_ms,
            project: project.to_string(),
            ..row(Agent::Claude, date, "m", 1)
        };
        let early = || at(100, "2026-01-01", "old");
        let late = || at(200, "2026-01-02", "new");
        let latest = |r: &Report| {
            let g = r.groups.values().next().unwrap();
            (g.project.clone(), g.last_date.as_str().to_string())
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
                row(Agent::Codex, "2026-01-01", "gpt-5.2", 10),
                row(Agent::Codex, "2026-01-01", "unpriced", 20),
                row(Agent::Claude, "2026-01-02", "claude-opus-5", 30),
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
                "{k:?}"
            );
        }
        assert_eq!(m.footer.unpriced_tokens, one.footer.unpriced_tokens);
    }
}
