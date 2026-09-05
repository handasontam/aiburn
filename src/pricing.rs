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
const fn c(input: f64, output: f64) -> Rate {
    c_read(input, output, input * 0.1)
}

/// Anthropic with an explicit cache-read rate, for models that break the usual
/// 10%-of-input rule while keeping the cache-write multipliers.
const fn c_read(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
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
/// emit. Premium (Fast/Priority) builds are `-fast` keys next to their base
/// model — OpenAI's are 2× Standard (2.5× for GPT-5.5), matching ccusage's
/// multipliers. See `resolve` for how a logged name matches these keys.
static TABLE: &[(&str, Rate)] = &[
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
    // Fable 5.1 reads cache at $0.25/MTok (2.5% of input, not the usual 10%).
    ("claude-fable-5-1", c_read(10.0, 50.0, 0.25)),
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
    // GPT-5.6 capability models (Sol, Terra, Luna) have distinct rates; the
    // bare name is aliased to the flagship Sol in `alias`.
    ("gpt-5.6-sol", o(5.0, 30.0, 0.5)),
    ("gpt-5.6-sol-fast", o(10.0, 60.0, 1.0)),
    ("gpt-5.6-terra", o(2.0, 12.0, 0.2)),
    ("gpt-5.6-terra-fast", o(4.0, 24.0, 0.4)),
    ("gpt-5.6-luna", o(0.2, 1.2, 0.02)),
    ("gpt-5.6-luna-fast", o(0.4, 2.4, 0.04)),
    ("gpt-5.5", o(5.0, 30.0, 0.5)),
    ("gpt-5.5-fast", o(12.5, 75.0, 1.25)),
    ("gpt-5.4", o(2.5, 15.0, 0.25)),
    ("gpt-5.4-fast", o(5.0, 30.0, 0.5)),
    ("gpt-5.4-mini", o(0.75, 4.5, 0.075)),
    ("gpt-5.3-codex", o(1.75, 14.0, 0.175)),
    ("gpt-5.3-codex-fast", o(3.5, 28.0, 0.35)),
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
];

const AUTO_REVIEW_LUNA_AT_MS: i64 = 1_785_369_600_000; // 2026-07-30T00:00:00Z

/// Bare aliases and virtual model names → a concrete pricing key.
fn alias(model: &str, at_ms: i64) -> Option<&'static str> {
    match model {
        "opus" => Some("claude-opus-5"),
        "sonnet" => Some("claude-sonnet-5"),
        "haiku" => Some("claude-haiku-4-5"),
        // Older logs omit the capability suffix; the bare name is the flagship.
        "gpt-5.6" => Some("gpt-5.6-sol"),
        // The raw name is retained in reports, but its underlying model
        // changed from GPT-5.5 to GPT-5.6-Luna on the public cutover date.
        "codex-auto-review" if at_ms >= AUTO_REVIEW_LUNA_AT_MS => Some("gpt-5.6-luna"),
        "codex-auto-review" => Some("gpt-5.5"),
        _ => None,
    }
}

fn lookup(key: &str, at_ms: i64) -> Option<&'static Rate> {
    let find = |key: &str| TABLE.iter().find(|(k, _)| *k == key).map(|(_, r)| r);
    if let Some(r) = find(key) {
        return Some(r);
    }
    if let Some(r) = alias(key, at_ms).and_then(find) {
        return Some(r);
    }
    // longest matching prefix (handles dated/regional suffixes)
    TABLE
        .iter()
        .filter(|(k, _)| key.starts_with(*k))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, r)| r)
}

/// Match a logged model name to its rate: exact, then bare alias, then
/// longest prefix (so dated/regional variants resolve to their base). A
/// `-fast` build we have no premium rate for retries as its base model, which
/// undercounts rather than dropping the row to $0.
fn resolve(model: &str, at_ms: i64) -> Option<&'static Rate> {
    let key = model.to_ascii_lowercase();
    lookup(&key, at_ms).or_else(|| lookup(key.strip_suffix("-fast")?, at_ms))
}

/// Returns (cost in USD, whether pricing was found). `at_ms` is when the usage
/// happened so date-based model aliases remain historically reproducible.
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
        assert_eq!(cost_for("gpt-5.6-sol", &t, 0), (35.5, true));
        assert_eq!(cost_for("gpt-5.6-terra", &t, 0), (14.2, true));
        assert_eq!(cost_for("gpt-5.6-luna", &t, 0), (1.42, true));
        // Bare 5.6 follows the flagship.
        assert_eq!(cost_for("gpt-5.6", &t, 0), (35.5, true));
    }

    #[test]
    fn fable_5_1_discounts_cache_reads() {
        let t = one_each();
        // $10 input + $50 output + $0.25 cache read per MTok.
        assert_eq!(cost_for("claude-fable-5-1", &t, 0), (60.25, true));
        // A dated build resolves to 5.1 by longest prefix, not to bare fable-5.
        assert_eq!(cost_for("claude-fable-5-1-20260901", &t, 0), (60.25, true));
        assert_eq!(cost_for("claude-fable-5", &t, 0), (61.0, true));
    }

    #[test]
    fn fast_keys_use_premium_rates() {
        let t = one_each();
        assert_eq!(cost_for("gpt-5.6-sol-fast", &t, 0), (71.0, true));
        assert_eq!(cost_for("gpt-5.6-luna-fast", &t, 0), (2.84, true));
        assert_eq!(cost_for("gpt-5.5-fast", &t, 0), (88.75, true));
        assert_eq!(cost_for("gpt-5.4-fast", &t, 0), (35.5, true));
    }

    #[test]
    fn unknown_fast_build_falls_back_to_base_rate() {
        let t = one_each();
        assert_eq!(cost_for("gpt-5.2-fast", &t, 0), cost_for("gpt-5.2", &t, 0));
    }

    #[test]
    fn auto_review_switches_to_luna_on_cutover() {
        let t = one_each();
        assert_eq!(cost_for("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS - 1), (35.5, true));
        assert_eq!(cost_for("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS), (1.42, true));
    }
}
