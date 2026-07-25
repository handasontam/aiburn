use std::sync::OnceLock;

use crate::model::Tokens;

/// Rates in USD per 1,000,000 tokens.
pub struct Rate {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,    // 5-minute cache creation
    pub cache_write_1h: f64, // 1-hour cache creation
    pub cache_read: f64,
}

/// Anthropic: 5m cache write = 1.25× input, 1h = 2× input, cache read = 0.1×.
fn c(input: f64, output: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read: input * 0.1,
    }
}

/// OpenAI: no separate cache-write; cached input has its own rate.
fn o(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write: 0.0,
        cache_write_1h: 0.0,
        cache_read,
    }
}

/// Curated pricing for the models Claude Code and Codex actually emit.
/// Sourced from LiteLLM / models.dev (2026-07). See `resolve_key` for how a
/// logged model name is matched against these keys.
fn table() -> &'static Vec<(&'static str, Rate)> {
    static T: OnceLock<Vec<(&'static str, Rate)>> = OnceLock::new();
    T.get_or_init(|| {
        vec![
            // --- Anthropic / Claude Code ---
            ("claude-opus-5", c(5.0, 25.0)),
            // Fast mode: same model, ~2.5× output speed at premium rates.
            ("claude-opus-5-fast", c(10.0, 50.0)),
            ("claude-opus-4-8", c(5.0, 25.0)),
            ("claude-opus-4-7", c(5.0, 25.0)),
            ("claude-opus-4-6", c(5.0, 25.0)),
            ("claude-opus-4-5", c(5.0, 25.0)),
            ("claude-opus-4-1", c(15.0, 75.0)),
            ("claude-opus-4", c(15.0, 75.0)),
            ("claude-fable-5", c(10.0, 50.0)),
            ("claude-sonnet-5", c(3.0, 15.0)), // launch rate until 2026-09-01; see `intro`
            ("claude-sonnet-4-6", c(3.0, 15.0)),
            ("claude-sonnet-4-5", c(3.0, 15.0)),
            ("claude-sonnet-4", c(3.0, 15.0)),
            ("claude-3-5-sonnet", c(3.0, 15.0)),
            ("claude-haiku-4-5", c(1.0, 5.0)),
            ("claude-3-5-haiku", c(0.8, 4.0)),
            ("claude-3-opus", c(15.0, 75.0)),
            ("claude-3-haiku", c(0.25, 1.25)),
            // --- OpenAI / Codex ---
            ("gpt-5.6", o(5.0, 30.0, 0.5)),
            ("gpt-5.5", o(5.0, 30.0, 0.5)),
            ("gpt-5.4", o(2.5, 15.0, 0.25)),
            ("gpt-5.3-codex", o(1.75, 14.0, 0.175)),
            ("gpt-5.3", o(1.75, 14.0, 0.175)),
            ("gpt-5.2-codex", o(1.75, 14.0, 0.175)),
            ("gpt-5.2", o(1.75, 14.0, 0.175)),
            ("gpt-5.1-codex-mini", o(0.25, 2.0, 0.025)),
            ("gpt-5.1-codex-max", o(1.25, 10.0, 0.125)),
            ("gpt-5.1-codex", o(1.25, 10.0, 0.125)),
            ("gpt-5.1", o(1.25, 10.0, 0.125)),
            ("gpt-5-codex", o(1.25, 10.0, 0.125)),
            ("gpt-5-mini", o(0.25, 2.0, 0.025)),
            ("gpt-5-nano", o(0.05, 0.4, 0.005)),
            ("gpt-5", o(1.25, 10.0, 0.125)),
            ("codex-mini-latest", o(1.5, 6.0, 0.375)),
            ("codex-mini", o(1.5, 6.0, 0.375)),
            ("o3", o(2.0, 8.0, 0.5)),
            ("o4-mini", o(1.1, 4.4, 0.275)),
        ]
    })
}

/// Promotional rates that expire, as (key, epoch-ms the promo ends, rate).
/// Checked before `table` and only for usage stamped *before* the cutoff, so
/// past days keep the price that actually applied on the day they ran.
///
/// Sonnet 5 launched at $2/$10; it goes to its standard $3/$15 on 2026-09-01
/// (cutoff taken as UTC midnight — Anthropic doesn't publish a zone).
fn intro() -> &'static Vec<(&'static str, i64, Rate)> {
    static T: OnceLock<Vec<(&'static str, i64, Rate)>> = OnceLock::new();
    T.get_or_init(|| vec![("claude-sonnet-5", 1_788_220_800_000, c(2.0, 10.0))])
}

/// Bare aliases and virtual model names → a concrete pricing key.
fn alias(model: &str) -> Option<&'static str> {
    match model {
        "opus" => Some("claude-opus-5"),
        "sonnet" => Some("claude-sonnet-5"),
        "haiku" => Some("claude-haiku-4-5"),
        // Codex review sub-agent runs on the current flagship model.
        "codex-auto-review" => Some("gpt-5.5"),
        _ => None,
    }
}

fn lookup(key: &str) -> Option<&'static str> {
    let t = table();
    if let Some((k, _)) = t.iter().find(|(k, _)| *k == key) {
        return Some(k);
    }
    if let Some(target) = alias(key) {
        if let Some((k, _)) = t.iter().find(|(k, _)| *k == target) {
            return Some(k);
        }
    }
    // longest matching prefix (handles dated/regional suffixes)
    t.iter()
        .filter(|(k, _)| key.starts_with(*k))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, _)| *k)
}

/// Match a logged model name to a pricing key: exact, then bare alias, then
/// longest prefix (so dated/regional variants resolve to their base). A
/// `-fast` build we have no premium rate for retries as its base model, which
/// undercounts rather than dropping the row to $0.
fn resolve_key(model: &str) -> Option<&'static str> {
    let key = model.to_ascii_lowercase();
    lookup(&key).or_else(|| lookup(key.strip_suffix("-fast")?))
}

fn resolve(model: &str, at_ms: i64) -> Option<&'static Rate> {
    let key = resolve_key(model)?;
    if let Some((_, _, r)) = intro().iter().find(|(k, until, _)| *k == key && at_ms < *until) {
        return Some(r);
    }
    table().iter().find(|(k, _)| *k == key).map(|(_, r)| r)
}

/// Returns (cost in USD, whether pricing was found). `at_ms` is when the usage
/// happened, so rows priced under an expired promo stay historically correct.
pub fn cost_for(model: &str, t: &Tokens, at_ms: i64) -> (f64, bool) {
    match resolve(model, at_ms) {
        None => (0.0, false),
        Some(r) => {
            let usd = (t.input as f64 * r.input
                + t.output as f64 * r.output
                + t.cache_write as f64 * r.cache_write
                + t.cache_write_1h as f64 * r.cache_write_1h
                + t.cache_read as f64 * r.cache_read)
                / 1_000_000.0;
            (usd, true)
        }
    }
}
