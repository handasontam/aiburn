//! Incremental aggregation. Usage records are folded into `Report` as they are
//! parsed and then dropped, so peak memory scales with the number of distinct
//! groups (days / months / sessions) plus the footer, not the number of events.

use std::collections::{BTreeSet, HashMap};

use crate::model::{Agent, UsageRow};

#[derive(Clone, Copy, PartialEq)]
pub enum Command {
    Daily,
    Monthly,
    Session,
}

/// The report request: how to group, and which rows to include.
pub struct Query {
    pub command: Command,
    pub agent: Option<Agent>,
    pub since: Option<String>,
    pub until: Option<String>,
}

impl Query {
    fn accepts(&self, agent: Agent, date: &str) -> bool {
        if let Some(a) = self.agent {
            if a != agent {
                return false;
            }
        }
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

    fn key(&self, r: &UsageRow) -> String {
        match self.command {
            Command::Monthly => r.month.clone(),
            Command::Session => format!("{}:{}", r.agent.as_str(), r.session_id),
            Command::Daily => r.date.clone(),
        }
    }
}

#[derive(Default)]
pub struct Group {
    pub key: String,
    pub claude_cost: f64,
    pub codex_cost: f64,
    pub input: u64,
    pub output: u64,
    pub cache: u64,
    pub models: BTreeSet<String>,
    pub agents: BTreeSet<&'static str>,
    pub project: String,
    pub last_ts: i64,
}

impl Group {
    pub fn total(&self) -> f64 {
        self.claude_cost + self.codex_cost
    }
}

#[derive(Default)]
pub struct Footer {
    pub claude_cost: f64,
    pub codex_cost: f64,
    pub claude_tokens: u64,
    pub codex_tokens: u64,
    pub unpriced_tokens: u64,
    pub unpriced_models: BTreeSet<String>,
}

#[derive(Default)]
pub struct Report {
    pub groups: HashMap<String, Group>,
    pub footer: Footer,
}

impl Report {
    /// Fold one usage record in (respecting the query filter), then let it drop.
    pub fn add(&mut self, q: &Query, r: &UsageRow) {
        if !q.accepts(r.agent, &r.date) {
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

        let key = q.key(r);
        let g = self.groups.entry(key.clone()).or_insert_with(|| Group {
            key,
            ..Default::default()
        });
        match r.agent {
            Agent::Claude => g.claude_cost += r.cost,
            Agent::Codex => g.codex_cost += r.cost,
        }
        g.input += r.tokens.input;
        g.output += r.tokens.output;
        g.cache += r.tokens.cache_read + r.tokens.cache_write + r.tokens.cache_write_1h;
        if !g.models.contains(&r.model) {
            g.models.insert(r.model.clone());
        }
        g.agents.insert(r.agent.as_str());
        if g.project.is_empty() && !r.project.is_empty() {
            g.project = r.project.clone();
        }
        if r.timestamp > g.last_ts {
            g.last_ts = r.timestamp;
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

        for (k, og) in other.groups {
            let g = self.groups.entry(k).or_insert_with(|| Group {
                key: og.key.clone(),
                ..Default::default()
            });
            g.claude_cost += og.claude_cost;
            g.codex_cost += og.codex_cost;
            g.input += og.input;
            g.output += og.output;
            g.cache += og.cache;
            g.models.extend(og.models);
            g.agents.extend(og.agents);
            if g.project.is_empty() {
                g.project = og.project;
            }
            if og.last_ts > g.last_ts {
                g.last_ts = og.last_ts;
            }
        }
        self
    }

    /// Groups sorted for display: by key (period) or by recency (session).
    pub fn sorted_groups(&self, command: Command) -> Vec<&Group> {
        let mut v: Vec<&Group> = self.groups.values().collect();
        match command {
            Command::Session => v.sort_by(|a, b| b.last_ts.cmp(&a.last_ts)),
            _ => v.sort_by(|a, b| a.key.cmp(&b.key)),
        }
        v
    }
}
