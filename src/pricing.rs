use std::borrow::Cow;

use crate::model::Tokens;

/// Rates in USD per 1,000,000 tokens.
pub struct Rate {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,    // 5-minute cache creation
    pub cache_write_1h: f64, // 1-hour cache creation
    pub cache_read: f64,
    /// OpenAI two-stage model: requests over `LONG_CONTEXT_THRESHOLD` bill at
    /// `LONG_CONTEXT_MULT` times these rates.
    pub long_context: bool,
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
        long_context: false,
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
        long_context: false,
    }
}

/// OpenAI model with a long-context tier (every GPT-5.4+ flagship).
const fn o_lc(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        long_context: true,
        ..o(input, output, cache_read)
    }
}

/// Curated public list pricing for the models Claude Code and Codex actually
/// emit. Premium (Fast/Priority) builds are `-fast` keys next to their base
/// model — OpenAI's are 2× Standard (2.5× for GPT-5.5), matching ccusage's
/// multipliers. See `resolve` for how a logged name matches these keys.
static TABLE: &[(&str, Rate)] = &[
    // --- Anthropic / Claude Code ---
    // Opus 5.5 is cheaper than Opus 5 and reads cache at 5% of input; its own
    // key keeps it from prefix-matching `claude-opus-5`.
    ("claude-opus-5-5", c_read(4.0, 20.0, 0.2)),
    ("claude-opus-5-5-fast", c_read(8.0, 40.0, 0.4)),
    ("claude-opus-5", c(5.0, 25.0)),
    // Fast mode: same model, ~2.5× output speed at premium rates.
    ("claude-opus-5-fast", c(10.0, 50.0)),
    ("claude-opus-4-8", c(5.0, 25.0)),
    ("claude-opus-4-8-fast", c(10.0, 50.0)),
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
    ("gpt-6-astra", o_lc(10.0, 50.0, 1.0)),
    ("gpt-6-astra-fast", o_lc(20.0, 100.0, 2.0)),
    ("gpt-6-sol", o_lc(2.0, 10.0, 0.2)),
    ("gpt-6-sol-fast", o_lc(4.0, 20.0, 0.4)),
    ("gpt-6-luna", o_lc(0.1, 0.5, 0.01)),
    ("gpt-6-luna-fast", o_lc(0.2, 1.0, 0.02)),
    // GPT-5.6 capability models (Sol, Terra, Luna) have distinct rates; the
    // bare name is aliased to the flagship Sol in `alias`.
    ("gpt-5.6-sol", o_lc(4.0, 20.0, 0.4)),
    ("gpt-5.6-sol-fast", o_lc(8.0, 40.0, 0.8)),
    ("gpt-5.6-terra", o_lc(2.0, 12.0, 0.2)),
    ("gpt-5.6-terra-fast", o_lc(4.0, 24.0, 0.4)),
    ("gpt-5.6-luna", o_lc(0.2, 1.2, 0.02)),
    ("gpt-5.6-luna-fast", o_lc(0.4, 2.4, 0.04)),
    ("gpt-5.5", o_lc(5.0, 30.0, 0.5)),
    ("gpt-5.5-fast", o_lc(12.5, 75.0, 1.25)),
    ("gpt-5.4", o_lc(2.5, 15.0, 0.25)),
    ("gpt-5.4-fast", o_lc(5.0, 30.0, 0.5)),
    ("gpt-5.4-mini", o(0.75, 4.5, 0.075)),
    ("gpt-5.4-mini-fast", o(1.5, 9.0, 0.15)),
    ("gpt-5.4-nano", o(0.2, 1.25, 0.02)),
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
    // Pro models publish no cached-input discount, so cached tokens bill as
    // fresh input; their own keys keep them off the base model's rate.
    ("gpt-5.5-pro", o_lc(30.0, 180.0, 30.0)),
    ("gpt-5.4-pro", o_lc(30.0, 180.0, 30.0)),
    ("gpt-5.2-pro", o(21.0, 168.0, 21.0)),
    ("gpt-5-pro", o(15.0, 120.0, 15.0)),
    ("o3-pro", o(20.0, 80.0, 20.0)),
];

/// OpenAI bills a request whose whole context (fresh + cached input) exceeds
/// this many tokens entirely at a higher tier — a per-request switch, not a
/// marginal breakpoint. Matches ccusage.
const LONG_CONTEXT_THRESHOLD: u64 = 272_000;

/// Long-context tier as (input, output, cache-read) multiples of the base
/// rate. Every tiered OpenAI model in models.dev's 2026-09-05 snapshot uses
/// exactly these, so the tier is derived rather than tabulated; a model that
/// breaks the pattern would need its own rates.
const LONG_CONTEXT_MULT: (f64, f64, f64) = (2.0, 1.5, 2.0);

/// Premium usage is priced through a `-fast` sibling key, so the service tier
/// is folded into the model name. Idempotent: some logs already carry it.
pub fn with_fast_suffix(model: String) -> String {
    if model.ends_with("-fast") {
        model
    } else {
        model + "-fast"
    }
}

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

fn find(key: &str) -> Option<(&'static str, &'static Rate)> {
    TABLE.iter().find(|(k, _)| *k == key).map(|(k, r)| (*k, r))
}

/// Returns the matched `TABLE` entry as (key, rate).
fn lookup(key: &str, at_ms: i64) -> Option<(&'static str, &'static Rate)> {
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
/// `-fast` build resolves its base name first and then takes that key's
/// premium sibling, so `claude-opus-5-20260301-fast` finds `claude-opus-5-fast`
/// instead of prefix-matching the standard rate. A base with no premium key
/// keeps its own rate, which undercounts rather than dropping the row to $0.
fn resolve(model: &str, at_ms: i64) -> Option<&'static Rate> {
    // Logged names are already lowercase; only allocate for the odd one out.
    let key: Cow<str> = if model.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(model.to_ascii_lowercase())
    } else {
        Cow::Borrowed(model)
    };
    let Some(base) = key.strip_suffix("-fast") else {
        return lookup(&key, at_ms).map(|(_, r)| r);
    };
    let (base_key, base_rate) = lookup(base, at_ms)?;
    Some(find(&format!("{base_key}-fast")).map_or(base_rate, |(_, r)| r))
}

/// Returns (cost in USD, whether pricing was found). `at_ms` is when the usage
/// happened so date-based model aliases remain historically reproducible.
pub fn cost_for(model: &str, t: &Tokens, at_ms: i64) -> (f64, bool) {
    let Some(r) = resolve(model, at_ms) else {
        return (0.0, false);
    };
    let context = t.input + t.cache_read + t.cache_write + t.cache_write_1h;
    let (mi, mo, mc) = if r.long_context && context > LONG_CONTEXT_THRESHOLD {
        LONG_CONTEXT_MULT
    } else {
        (1.0, 1.0, 1.0)
    };
    let usd = (t.input as f64 * r.input * mi
        + t.output as f64 * r.output * mo
        + t.cache_write as f64 * r.cache_write * mi
        + t.cache_write_1h as f64 * r.cache_write_1h * mi
        + t.cache_read as f64 * r.cache_read * mc)
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
        assert_eq!(usd("gpt-6-astra", &t, 0), 51.1);
        assert_eq!(usd("gpt-6-sol", &t, 0), 10.22);
        assert_eq!(usd("gpt-6-luna", &t, 0), 0.511);
        // Bare names follow the flagship of their generation.
        assert_eq!(usd("gpt-5.6", &t, 0), 20.44);
        assert_eq!(usd("gpt-6", &t, 0), 51.1);
    }

    #[test]
    fn claude_cache_writes_use_tier_multipliers() {
        // 5-minute writes at 1.25× input, 1-hour at 2×: asymmetric so a swap
        // of the two fields is visible.
        let t = Tokens {
            cache_write: 1_000_000,
            cache_write_1h: 1_000_000,
            ..Default::default()
        };
        assert_eq!(usd("claude-opus-5", &t, 0), 16.25);
    }

    #[test]
    fn long_context_request_bills_whole_request_at_tier_rate() {
        // 500K fresh input + 1M output on Astra: every token at $20/$75.
        let long = Tokens {
            input: 500_000,
            output: 1_000_000,
            ..Default::default()
        };
        assert_eq!(usd("gpt-6-astra", &long, 0), 85.0);
        // Fast keeps its 2× over the tier.
        assert_eq!(usd("gpt-5.6-sol-fast", &long, 0), 68.0);
        // Cached input counts toward the context that selects the tier.
        let cached = Tokens {
            input: 250_000,
            cache_read: 250_000,
            ..Default::default()
        };
        assert_eq!(usd("gpt-6-astra", &cached, 0), 5.5);
        // Exactly at the threshold stays on the base rate.
        let boundary = Tokens {
            input: LONG_CONTEXT_THRESHOLD,
            ..Default::default()
        };
        assert_eq!(usd("gpt-6-astra", &boundary, 0), 2.72);
    }

    #[test]
    fn long_context_tier_is_per_model() {
        let long = Tokens {
            input: 500_000,
            output: 1_000_000,
            ..Default::default()
        };
        // gpt-5.4-mini has its own untiered entry; it must not resolve to
        // gpt-5.4 and pick up the surcharge.
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
    fn opus_5_5_does_not_resolve_to_opus_5() {
        let t = short();
        // $4 input + $20 output + $0.20 cache read per MTok.
        assert_eq!(usd("claude-opus-5-5", &t, 0), 20.42);
        assert_eq!(usd("claude-opus-5-5-20261001", &t, 0), 20.42);
        assert_eq!(usd("claude-opus-5-5-20261001-fast", &t, 0), 40.84);
        assert_eq!(usd("claude-opus-5", &t, 0), 25.55);
    }

    #[test]
    fn fast_resolution() {
        let t = short();
        // An exact `-fast` key wins over stripping the suffix.
        assert_eq!(usd("gpt-5.6-sol-fast", &t, 0), 40.88);
        assert_eq!(usd("gpt-5.5-fast", &t, 0), 76.375);
        // Dated and aliased names find the premium sibling of their resolved
        // base key instead of prefix-matching the standard rate.
        assert_eq!(
            usd("claude-opus-5-20260301-fast", &t, 0),
            usd("claude-opus-5-fast", &t, 0)
        );
        assert_eq!(usd("gpt-6-fast", &t, 0), 102.2);
        let at = AUTO_REVIEW_LUNA_AT_MS;
        assert_eq!(
            usd("codex-auto-review-fast", &t, at),
            usd("gpt-5.6-luna-fast", &t, at)
        );
        // A base with no premium key keeps its own rate rather than $0.
        assert_eq!(usd("gpt-5.2-fast", &t, 0), usd("gpt-5.2", &t, 0));
    }

    #[test]
    fn auto_review_switches_to_luna_on_cutover() {
        let t = short();
        assert_eq!(
            usd("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS - 1),
            30.55
        );
        assert_eq!(usd("codex-auto-review", &t, AUTO_REVIEW_LUNA_AT_MS), 1.222);
    }

    /// Every rate models.dev publishes must match the one `resolve` picks, so a
    /// missing row, a prefix match onto the wrong model, or a missing
    /// long-context tier all surface. Needs a fresh snapshot, so it is ignored
    /// here and run daily by `pricing-drift.yml`; locally:
    ///
    /// ```sh
    /// curl -fsSL https://models.dev/api.json | jq -rf .github/models-dev.jq > /tmp/models-dev.tsv
    /// MODELS_DEV_TSV=/tmp/models-dev.tsv cargo test matches_models_dev -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn matches_models_dev() {
        let path = std::env::var("MODELS_DEV_TSV").expect("MODELS_DEV_TSV names the jq output");
        let tsv = std::fs::read_to_string(path).expect("read MODELS_DEV_TSV");
        let mut drift = Vec::new();
        for line in tsv.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [provider, model, input, output, cache_read, cache_write, lc_size, lc_input, lc_output, lc_cache_read] =
                f[..]
            else {
                panic!("malformed row: {line}");
            };
            let num = |s: &str| {
                (!s.is_empty()).then(|| {
                    s.parse::<f64>()
                        .unwrap_or_else(|_| panic!("bad number in: {line}"))
                })
            };
            let Some(r) = resolve(model, 0) else {
                drift.push(format!("{model}: not priced"));
                continue;
            };
            // A rate models.dev leaves empty (Pro models have no cached-input
            // rate) has nothing to compare against.
            let mut pairs = vec![
                ("input", r.input, num(input)),
                ("output", r.output, num(output)),
                ("cache read", r.cache_read, num(cache_read)),
            ];
            match provider {
                "anthropic" => pairs.push(("cache write", r.cache_write, num(cache_write))),
                // Codex logs carry no cache-write tokens, so that rate is never billed.
                "openai" => {}
                _ => panic!("unexpected provider in: {line}"),
            }
            // models.dev publishes no long-context tier for Fast modes.
            if !model.ends_with("-fast") {
                match (r.long_context, num(lc_size)) {
                    (false, None) => {}
                    (true, None) => {
                        drift.push(format!("{model}: long-context tier, models.dev has none"))
                    }
                    (false, Some(_)) => {
                        drift.push(format!("{model}: no long-context tier, models.dev has one"))
                    }
                    (true, size) => {
                        let (mi, mo, mc) = LONG_CONTEXT_MULT;
                        pairs.extend([
                            (
                                "long-context threshold",
                                LONG_CONTEXT_THRESHOLD as f64,
                                size,
                            ),
                            ("long-context input", r.input * mi, num(lc_input)),
                            ("long-context output", r.output * mo, num(lc_output)),
                            (
                                "long-context cache read",
                                r.cache_read * mc,
                                num(lc_cache_read),
                            ),
                        ]);
                    }
                }
            }
            for (what, ours, theirs) in pairs {
                if let Some(theirs) = theirs.filter(|t| (ours - t).abs() > 1e-9) {
                    drift.push(format!("{model}: {what} {ours}, models.dev {theirs}"));
                }
            }
        }
        assert!(
            drift.is_empty(),
            "{} differences from models.dev:\n{}",
            drift.len(),
            drift.join("\n")
        );
    }
}
