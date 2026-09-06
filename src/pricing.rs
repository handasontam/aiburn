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
    ("gpt-6-astra", o(10.0, 50.0, 1.0)),
    ("gpt-6-astra-fast", o(20.0, 100.0, 2.0)),
    // GPT-5.6 capability models (Sol, Terra, Luna) have distinct rates; the
    // bare name is aliased to the flagship Sol in `alias`.
    ("gpt-5.6-sol", o(4.0, 20.0, 0.4)),
    ("gpt-5.6-sol-fast", o(8.0, 40.0, 0.8)),
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

/// OpenAI bills a request whose whole context (fresh + cached input) exceeds
/// this many tokens entirely at a higher tier — a per-request switch, not a
/// marginal breakpoint. Matches ccusage.
const LONG_CONTEXT_THRESHOLD: u64 = 272_000;

/// Long-context tier rates, keyed by the exact `TABLE` key a model resolved
/// to. Exact only: a prefix match here would graft a parent's surcharge onto
/// a variant that has none (gpt-5.4-mini). Fast keys carry the same multiple
/// over the tier as over the base rate.
static LONG_CONTEXT: &[(&str, Rate)] = &[
    ("gpt-6-astra", o(20.0, 75.0, 2.0)),
    ("gpt-6-astra-fast", o(40.0, 150.0, 4.0)),
    ("gpt-5.6-sol", o(8.0, 30.0, 0.8)),
    ("gpt-5.6-sol-fast", o(16.0, 60.0, 1.6)),
    ("gpt-5.6-terra", o(4.0, 18.0, 0.4)),
    ("gpt-5.6-terra-fast", o(8.0, 36.0, 0.8)),
    ("gpt-5.6-luna", o(0.4, 1.8, 0.04)),
    ("gpt-5.6-luna-fast", o(0.8, 3.6, 0.08)),
    ("gpt-5.5", o(10.0, 45.0, 1.0)),
    ("gpt-5.5-fast", o(25.0, 112.5, 2.5)),
    ("gpt-5.4", o(5.0, 22.5, 0.5)),
    ("gpt-5.4-fast", o(10.0, 45.0, 1.0)),
];

const AUTO_REVIEW_LUNA_AT_MS: i64 = 1_785_369_600_000; // 2026-07-30T00:00:00Z

/// Bare aliases and virtual model names → a concrete pricing key.
fn alias(model: &str, at_ms: i64) -> Option<&'static str> {
    match model {
        "opus" => Some("claude-opus-5"),
        "sonnet" => Some("claude-sonnet-5"),
        "haiku" => Some("claude-haiku-4-5"),
        // Older logs omit the capability suffix; the bare name is the flagship.
        "gpt-6" => Some("gpt-6-astra"),
        "gpt-5.6" => Some("gpt-5.6-sol"),
        // The raw name is retained in reports, but its underlying model
        // changed from GPT-5.5 to GPT-5.6-Luna on the public cutover date.
        "codex-auto-review" if at_ms >= AUTO_REVIEW_LUNA_AT_MS => Some("gpt-5.6-luna"),
        "codex-auto-review" => Some("gpt-5.5"),
        _ => None,
    }
}

/// Returns the matched `TABLE` entry as (key, rate); the key also indexes
/// `LONG_CONTEXT`.
fn lookup(key: &str, at_ms: i64) -> Option<(&'static str, &'static Rate)> {
    let find = |key: &str| TABLE.iter().find(|(k, _)| *k == key).map(|(k, r)| (*k, r));
    if let Some(hit) = find(key) {
        return Some(hit);
    }
    if let Some(hit) = alias(key, at_ms).and_then(find) {
        return Some(hit);
    }
    // longest matching prefix (handles dated/regional suffixes)
    TABLE
        .iter()
        .filter(|(k, _)| key.starts_with(*k))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, r)| (*k, r))
}

/// Match a logged model name to its rate: exact, then bare alias, then
/// longest prefix (so dated/regional variants resolve to their base). A
/// `-fast` build we have no premium rate for retries as its base model, which
/// undercounts rather than dropping the row to $0.
fn resolve(model: &str, at_ms: i64) -> Option<(&'static str, &'static Rate)> {
    let key = model.to_ascii_lowercase();
    lookup(&key, at_ms).or_else(|| lookup(key.strip_suffix("-fast")?, at_ms))
}

/// Returns (cost in USD, whether pricing was found). `at_ms` is when the usage
/// happened so date-based model aliases remain historically reproducible.
pub fn cost_for(model: &str, t: &Tokens, at_ms: i64) -> (f64, bool) {
    let Some((key, base)) = resolve(model, at_ms) else {
        return (0.0, false);
    };
    let context = t.input + t.cache_read + t.cache_write + t.cache_write_1h;
    let r = if context > LONG_CONTEXT_THRESHOLD {
        LONG_CONTEXT.iter().find(|(k, _)| *k == key).map_or(base, |(_, r)| r)
    } else {
        base
    };
    let usd = (t.input as f64 * r.input
        + t.output as f64 * r.output
        + t.cache_write as f64 * r.cache_write
        + t.cache_write_1h as f64 * r.cache_write_1h
        + t.cache_read as f64 * r.cache_read)
        / 1_000_000.0;
    (usd, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 100K fresh input, 1M output, 100K cache reads: a 200K context, below
    /// the long-context threshold, so these exercise the base rates.
    fn short() -> Tokens {
        Tokens {
            input: 100_000,
            output: 1_000_000,
            cache_read: 100_000,
            ..Default::default()
        }
    }

    /// Cost in USD rounded to the micro-dollar so decimal literals compare exactly.
    fn usd(model: &str, t: &Tokens, at_ms: i64) -> f64 {
        let (cost, priced) = cost_for(model, t, at_ms);
        assert!(priced, "{model} should be priced");
        (cost * 1e6).round() / 1e6
    }

    #[test]
    fn capability_models_use_distinct_standard_rates() {
        let t = short();
        assert_eq!(usd("gpt-5.6-sol", &t, 0), 20.44);
        assert_eq!(usd("gpt-5.6-terra", &t, 0), 12.22);
        assert_eq!(usd("gpt-5.6-luna", &t, 0), 1.222);
        // Bare 5.6 follows the flagship.
        assert_eq!(usd("gpt-5.6", &t, 0), 20.44);
    }

    #[test]
    fn gpt_6_astra_rates() {
        let t = short();
        // $10 input + $50 output + $1 cache read per MTok; Fast is 2×.
        assert_eq!(usd("gpt-6-astra", &t, 0), 51.1);
        assert_eq!(usd("gpt-6-astra-fast", &t, 0), 102.2);
        assert_eq!(usd("gpt-6", &t, 0), 51.1);
    }

    #[test]
    fn long_context_request_bills_whole_request_at_tier_rate() {
        // 500K fresh input + 1M output on Astra: every token at $20/$75.
        let long = Tokens { input: 500_000, output: 1_000_000, ..Default::default() };
        assert_eq!(usd("gpt-6-astra", &long, 0), 85.0);
        // Fast keeps its 2× over the tier.
        assert_eq!(usd("gpt-5.6-sol-fast", &long, 0), 68.0);
        // Cached input counts toward the context that selects the tier.
        let cached = Tokens { input: 250_000, cache_read: 250_000, ..Default::default() };
        assert_eq!(usd("gpt-6-astra", &cached, 0), 5.5);
        // Exactly at the threshold stays on the base rate.
        let boundary = Tokens { input: LONG_CONTEXT_THRESHOLD, ..Default::default() };
        assert_eq!(usd("gpt-6-astra", &boundary, 0), 2.72);
    }

    #[test]
    fn long_context_tier_is_not_inherited_by_prefix() {
        let long = Tokens { input: 500_000, output: 1_000_000, ..Default::default() };
        // gpt-5.4-mini has no tier; it must not pick up gpt-5.4's surcharge.
        assert_eq!(usd("gpt-5.4-mini", &long, 0), 4.875);
        // Dated variants of a tiered model do get the tier via their base key.
        assert_eq!(usd("gpt-5.5-2026-04-23", &long, 0), 50.0);
    }

    #[test]
    fn fable_5_1_discounts_cache_reads() {
        let t = short();
        // $10 input + $50 output + $0.25 cache read per MTok.
        assert_eq!(usd("claude-fable-5-1", &t, 0), 51.025);
        // A dated build resolves to 5.1 by longest prefix, not to bare fable-5.
        assert_eq!(usd("claude-fable-5-1-20260901", &t, 0), 51.025);
        assert_eq!(usd("claude-fable-5", &t, 0), 51.1);
    }

    #[test]
    fn fast_keys_use_premium_rates() {
        let t = short();
        assert_eq!(usd("gpt-5.6-sol-fast", &t, 0), 40.88);
        assert_eq!(usd("gpt-5.6-luna-fast", &t, 0), 2.444);
        assert_eq!(usd("gpt-5.5-fast", &t, 0), 76.375);
        assert_eq!(usd("gpt-5.4-fast", &t, 0), 30.55);
    }

    #[test]
    fn unknown_fast_build_falls_back_to_base_rate() {
        let t = short();
        assert_eq!(cost_for("gpt-5.2-fast", &t, 0), cost_for("gpt-5.2", &t, 0));
    }

    #[test]
    fn auto_review_switches_to_luna_on_cutover() {
        let t = short();
        assert_eq!(usd("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS - 1), 30.55);
        assert_eq!(usd("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS), 1.222);
    }
}
