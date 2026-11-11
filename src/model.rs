use crate::time::Day;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ServiceTier {
    #[default]
    Standard,
    Fast,
}

impl ServiceTier {
    /// Codex calls the premium tier both `fast` and `priority`.
    pub fn from_name(name: &str) -> Self {
        match name {
            "fast" | "priority" => ServiceTier::Fast,
            _ => ServiceTier::Standard,
        }
    }
}

#[derive(Default)]
pub struct Tokens {
    /// Excludes cache reads, which Codex logs count as input.
    pub input: u64,
    /// Includes Codex reasoning tokens.
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }
}

pub struct UsageRow {
    pub agent: Agent,
    pub at_ms: i64,
    pub date: Day,
    pub session_id: String,
    pub project: String,
    pub model: String,
    pub tokens: Tokens,
}
