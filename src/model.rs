#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Agent {
    #[default]
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

#[derive(Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Clone, Default)]
pub struct Tokens {
    /// Non-cached input tokens (billed at input rate).
    pub input: u64,
    /// Output tokens (includes reasoning tokens for Codex).
    pub output: u64,
    /// 5-minute cache-creation tokens (Claude; billed at 1.25× input).
    pub cache_write: u64,
    /// 1-hour cache-creation tokens (Claude; billed at 2× input).
    pub cache_write_1h: u64,
    /// Cache-read tokens (billed at the discounted cache rate).
    pub cache_read: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_write_1h + self.cache_read
    }
}

pub struct UsageRow {
    pub agent: Agent,
    /// Epoch milliseconds.
    pub timestamp: i64,
    /// YYYY-MM-DD in local time.
    pub date: String,
    /// YYYY-MM in local time.
    pub month: String,
    pub session_id: String,
    pub project: String,
    pub model: String,
    pub tokens: Tokens,
    pub cost: f64,
    /// False when no pricing was found for the model (cost is 0).
    pub priced: bool,
}
