use std::sync::OnceLock;

use crate::model::{ServiceTier, Tokens};

/// Rates in USD per 1,000,000 tokens.
pub struct Rate {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,    // 5-minute cache creation
    pub cache_write_1h: f64, // 1-hour cache creation
    pub cache_read: f64,
}

/// Anthropic: 5m cache write = 1.25× input, 1h = 2× input, cache read = 0.1×.
const fn c(input: f64, output: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read: input * 0.1,
    }
}

/// OpenAI: no separate cache-write; cached input has its own rate.
const fn o(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write: 0.0,
        cache_write_1h: 0.0,
        cache_read,
    }
}

/// Curated public list pricing for the models Claude Code and Codex actually
/// emit. OpenAI's Codex Fast rates are per-model rather than a blanket 2×
/// multiplier; see `fast_rate` below. See `resolve_key` for model matching.
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
            ("claude-sonnet-5", c(2.0, 10.0)),
            ("claude-sonnet-4-6", c(3.0, 15.0)),
            ("claude-sonnet-4-5", c(3.0, 15.0)),
            ("claude-sonnet-4", c(3.0, 15.0)),
            ("claude-3-5-sonnet", c(3.0, 15.0)),
            ("claude-haiku-4-5", c(1.0, 5.0)),
            ("claude-3-5-haiku", c(0.8, 4.0)),
            ("claude-3-opus", c(15.0, 75.0)),
            ("claude-3-haiku", c(0.25, 1.25)),
            // --- OpenAI / Codex ---
            // OpenAI GPT-5.6 family: Sol, Terra, and Luna have distinct
            // standard rates. Keep the bare name for older logs that omit
            // the capability suffix; it follows the flagship Sol rate.
            ("gpt-5.6-sol", o(5.0, 30.0, 0.5)),
            ("gpt-5.6-terra", o(2.0, 12.0, 0.2)),
            ("gpt-5.6-luna", o(0.2, 1.2, 0.02)),
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

/// OpenAI Fast/Priority pricing, in USD per 1,000,000 tokens.
///
/// These are the published GPT-5.6 Fast rates. A known model without a
/// separate Fast rate falls back to its Standard rate rather than silently
/// becoming unpriced.
fn fast_rate(key: &str) -> Option<&'static Rate> {
    static SOL: Rate = o(8.0, 40.0, 0.8);
    static TERRA: Rate = o(4.0, 24.0, 0.4);
    static LUNA: Rate = o(0.4, 2.4, 0.04);

    match key {
        "gpt-5.6-sol" | "gpt-5.6" | "gpt-5.5" => Some(&SOL),
        "gpt-5.6-terra" => Some(&TERRA),
        "gpt-5.6-luna" => Some(&LUNA),
        _ => None,
    }
}

const AUTO_REVIEW_LUNA_AT_MS: i64 = 1_785_369_600_000; // 2026-07-30T00:00:00Z

/// Bare aliases and virtual model names → a concrete pricing key.
fn alias(model: &str, at_ms: i64) -> Option<&'static str> {
    match model {
        "opus" => Some("claude-opus-5"),
        "sonnet" => Some("claude-sonnet-5"),
        "haiku" => Some("claude-haiku-4-5"),
        // The raw name is retained in reports, but its underlying model
        // changed from GPT-5.5 to GPT-5.6-Luna on the public cutover date.
        "codex-auto-review" if at_ms >= AUTO_REVIEW_LUNA_AT_MS => Some("gpt-5.6-luna"),
        "codex-auto-review" => Some("gpt-5.5"),
        _ => None,
    }
}

fn lookup(key: &str, at_ms: i64) -> Option<(&'static str, &'static Rate)> {
    let t = table();
    let find = |key: &str| t.iter().find(|(k, _)| *k == key).map(|(k, r)| (*k, r));
    if let Some(hit) = find(key) {
        return Some(hit);
    }
    if let Some(hit) = alias(key, at_ms).and_then(find) {
        return Some(hit);
    }
    // longest matching prefix (handles dated/regional suffixes)
    t.iter()
        .filter(|(k, _)| key.starts_with(*k))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, r)| (*k, r))
}

/// Match a logged model name to its rate: exact, then bare alias, then
/// longest prefix (so dated/regional variants resolve to their base). A
/// `-fast` build we have no premium rate for retries as its base model, which
/// undercounts rather than dropping the row to $0.
fn resolve(model: &str, at_ms: i64, service_tier: ServiceTier) -> Option<&'static Rate> {
    let key = model.to_ascii_lowercase();
    let (key, standard) =
        lookup(&key, at_ms).or_else(|| lookup(key.strip_suffix("-fast")?, at_ms))?;
    if service_tier == ServiceTier::Fast {
        return Some(fast_rate(key).unwrap_or(standard));
    }
    Some(standard)
}

/// Returns (cost in USD, whether pricing was found). `at_ms` is when the usage
/// happened so date-based model aliases remain historically reproducible.
pub fn cost_for(model: &str, t: &Tokens, at_ms: i64, service_tier: ServiceTier) -> (f64, bool) {
    match resolve(model, at_ms, service_tier) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn one_each() -> Tokens {
        Tokens {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            ..Default::default()
        }
    }

    #[test]
    fn capability_models_use_distinct_standard_rates() {
        let t = one_each();
        assert_eq!(cost_for("gpt-5.6-sol", &t, 0, ServiceTier::Standard), (35.5, true));
        assert_eq!(cost_for("gpt-5.6-terra", &t, 0, ServiceTier::Standard), (14.2, true));
        assert_eq!(cost_for("gpt-5.6-luna", &t, 0, ServiceTier::Standard), (1.42, true));
    }

    #[test]
    fn fast_rates_are_model_specific() {
        let t = one_each();
        assert_eq!(cost_for("gpt-5.6-sol", &t, 0, ServiceTier::Fast), (48.8, true));
        assert_eq!(cost_for("gpt-5.6-luna", &t, 0, ServiceTier::Fast), (2.84, true));
    }

    #[test]
    fn auto_review_switches_to_luna_on_cutover() {
        let t = one_each();
        assert_eq!(cost_for("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS - 1, ServiceTier::Standard), (35.5, true));
        assert_eq!(cost_for("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS, ServiceTier::Standard), (1.42, true));
    }
}
