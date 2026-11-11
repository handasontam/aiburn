use std::borrow::Cow;

use crate::model::{ServiceTier, Tokens};
use crate::time::days_from_civil;

/// Rates in USD per 1,000,000 tokens.
pub struct Rate {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub long_context: Option<LongContext>,
}

/// Multiples of the base rates for a whole request over `threshold`.
#[derive(Clone, Copy)]
pub struct LongContext {
    pub threshold: u64,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
}

/// Every tiered OpenAI model shares this tier, so it is derived rather than
/// tabulated per model.
const OPENAI_LONG_CONTEXT: LongContext = LongContext {
    threshold: 272_000,
    input: 2.0,
    output: 1.5,
    cache_read: 2.0,
};

const HAIKU_5_5_LONG_CONTEXT: LongContext = LongContext {
    threshold: 100_000,
    input: 5.0,
    output: 5.0,
    cache_read: 5.0,
};

const fn anthropic(input: f64, output: f64) -> Rate {
    anthropic_cached(input, output, input * 0.1)
}

const fn anthropic_cached(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
        long_context: None,
    }
}

const fn openai(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        input,
        output,
        cache_write_5m: 0.0,
        cache_write_1h: 0.0,
        cache_read,
        long_context: None,
    }
}

const fn openai_long(input: f64, output: f64, cache_read: f64) -> Rate {
    Rate {
        long_context: Some(OPENAI_LONG_CONTEXT),
        ..openai(input, output, cache_read)
    }
}

/// Premium (Fast/Priority) rates are tabulated as `-fast` keys rather than
/// derived because the premium is not one multiple (1.75× for o3, 2.5× for
/// GPT-5.5).
static TABLE: &[(&str, Rate)] = &[
    ("claude-opus-5-5", anthropic_cached(4.0, 20.0, 0.2)),
    ("claude-opus-5-5-fast", anthropic_cached(8.0, 40.0, 0.4)),
    ("claude-opus-5", anthropic(5.0, 25.0)),
    ("claude-opus-5-fast", anthropic(10.0, 50.0)),
    ("claude-opus-4-8", anthropic(5.0, 25.0)),
    ("claude-opus-4-8-fast", anthropic(10.0, 50.0)),
    ("claude-opus-4-7", anthropic(5.0, 25.0)),
    ("claude-opus-4-6", anthropic(5.0, 25.0)),
    ("claude-opus-4-5", anthropic(5.0, 25.0)),
    ("claude-opus-4-1", anthropic(15.0, 75.0)),
    ("claude-opus-4", anthropic(15.0, 75.0)),
    ("claude-fable-5-2", anthropic_cached(8.0, 40.0, 0.2)),
    ("claude-fable-5-2-fast", anthropic_cached(16.0, 80.0, 0.4)),
    ("claude-fable-5-1", anthropic_cached(10.0, 50.0, 0.25)),
    ("claude-fable-5", anthropic(10.0, 50.0)),
    ("claude-sonnet-5-5", anthropic_cached(2.0, 10.0, 0.1)),
    ("claude-sonnet-5", anthropic(2.0, 10.0)),
    ("claude-sonnet-4-6", anthropic(3.0, 15.0)),
    ("claude-sonnet-4-5", anthropic(3.0, 15.0)),
    ("claude-sonnet-4", anthropic(3.0, 15.0)),
    ("claude-3-5-sonnet", anthropic(3.0, 15.0)),
    (
        "claude-haiku-5-5",
        Rate {
            long_context: Some(HAIKU_5_5_LONG_CONTEXT),
            ..anthropic(0.1, 0.5)
        },
    ),
    ("claude-haiku-4-5", anthropic(1.0, 5.0)),
    ("claude-3-5-haiku", anthropic(0.8, 4.0)),
    ("claude-3-opus", anthropic(15.0, 75.0)),
    ("claude-3-haiku", anthropic(0.25, 1.25)),
    ("gpt-6.1-sol", openai_long(2.0, 10.0, 0.1)),
    ("gpt-6.1-sol-fast", openai_long(4.0, 20.0, 0.2)),
    ("gpt-6.1-terra", openai_long(1.0, 5.0, 0.05)),
    ("gpt-6.1-terra-fast", openai_long(2.0, 10.0, 0.1)),
    ("gpt-6-astra", openai_long(10.0, 50.0, 1.0)),
    ("gpt-6-astra-fast", openai_long(20.0, 100.0, 2.0)),
    ("gpt-6-sol", openai_long(2.0, 10.0, 0.2)),
    ("gpt-6-sol-fast", openai_long(4.0, 20.0, 0.4)),
    ("gpt-6-luna", openai_long(0.1, 0.5, 0.01)),
    ("gpt-6-luna-fast", openai_long(0.2, 1.0, 0.02)),
    ("gpt-5.6-sol", openai_long(4.0, 20.0, 0.4)),
    ("gpt-5.6-sol-fast", openai_long(8.0, 40.0, 0.8)),
    ("gpt-5.6-terra", openai_long(2.0, 12.0, 0.2)),
    ("gpt-5.6-terra-fast", openai_long(4.0, 24.0, 0.4)),
    ("gpt-5.6-luna", openai_long(0.2, 1.2, 0.02)),
    ("gpt-5.6-luna-fast", openai_long(0.4, 2.4, 0.04)),
    ("gpt-5.5", openai_long(5.0, 30.0, 0.5)),
    ("gpt-5.5-fast", openai_long(12.5, 75.0, 1.25)),
    ("gpt-5.4", openai_long(2.5, 15.0, 0.25)),
    ("gpt-5.4-fast", openai_long(5.0, 30.0, 0.5)),
    ("gpt-5.4-mini", openai(0.75, 4.5, 0.075)),
    ("gpt-5.4-mini-fast", openai(1.5, 9.0, 0.15)),
    ("gpt-5.4-nano", openai(0.2, 1.25, 0.02)),
    ("gpt-5.3-codex", openai(1.75, 14.0, 0.175)),
    ("gpt-5.3-codex-fast", openai(3.5, 28.0, 0.35)),
    ("gpt-5.3", openai(1.75, 14.0, 0.175)),
    ("gpt-5.2-codex", openai(1.75, 14.0, 0.175)),
    ("gpt-5.2", openai(1.75, 14.0, 0.175)),
    ("gpt-5.2-fast", openai(3.5, 28.0, 0.35)),
    ("gpt-5.1-codex-mini", openai(0.25, 2.0, 0.025)),
    ("gpt-5.1-codex-max", openai(1.25, 10.0, 0.125)),
    ("gpt-5.1-codex", openai(1.25, 10.0, 0.125)),
    ("gpt-5.1", openai(1.25, 10.0, 0.125)),
    ("gpt-5.1-fast", openai(2.5, 20.0, 0.25)),
    ("gpt-5-codex", openai(1.25, 10.0, 0.125)),
    ("gpt-5-mini", openai(0.25, 2.0, 0.025)),
    ("gpt-5-mini-fast", openai(0.45, 3.6, 0.045)),
    ("gpt-5-nano", openai(0.05, 0.4, 0.005)),
    ("gpt-5", openai(1.25, 10.0, 0.125)),
    ("gpt-5-fast", openai(2.5, 20.0, 0.25)),
    ("codex-mini-latest", openai(1.5, 6.0, 0.375)),
    ("codex-mini", openai(1.5, 6.0, 0.375)),
    ("o3", openai(2.0, 8.0, 0.5)),
    ("o3-fast", openai(3.5, 14.0, 0.875)),
    ("o4-mini", openai(1.1, 4.4, 0.275)),
    ("o4-mini-fast", openai(2.0, 8.0, 0.5)),
    ("gpt-5.5-pro", openai_long(30.0, 180.0, 30.0)),
    ("gpt-5.4-pro", openai_long(30.0, 180.0, 30.0)),
    ("gpt-5.2-pro", openai(21.0, 168.0, 21.0)),
    ("gpt-5-pro", openai(15.0, 120.0, 15.0)),
    ("o3-pro", openai(20.0, 80.0, 20.0)),
];

/// Premium usage is priced through a `-fast` sibling key. Some logged names
/// already carry the suffix.
pub fn tiered_model(model: String, tier: ServiceTier) -> String {
    match tier {
        ServiceTier::Fast if !model.ends_with("-fast") => model + "-fast",
        _ => model,
    }
}

const AUTO_REVIEW_LUNA_AT_MS: i64 = days_from_civil(2026, 7, 30) * 86_400_000;

fn alias(model: &str, at_ms: i64) -> Option<&'static str> {
    match model {
        "opus" => Some("claude-opus-5"),
        "sonnet" => Some("claude-sonnet-5"),
        "haiku" => Some("claude-haiku-4-5"),
        // Older logs omit the capability suffix; the bare name is the flagship.
        "gpt-6" => Some("gpt-6-astra"),
        "gpt-5.6" => Some("gpt-5.6-sol"),
        // Reports keep the raw name; the model behind it changed on this date.
        "codex-auto-review" if at_ms >= AUTO_REVIEW_LUNA_AT_MS => Some("gpt-5.6-luna"),
        "codex-auto-review" => Some("gpt-5.5"),
        _ => None,
    }
}

fn find(key: &str) -> Option<(&'static str, &'static Rate)> {
    TABLE.iter().find(|(k, _)| *k == key).map(|(k, r)| (*k, r))
}

fn lookup(key: &str, at_ms: i64) -> Option<(&'static str, &'static Rate)> {
    if let Some(hit) = find(key) {
        return Some(hit);
    }
    if let Some(hit) = alias(key, at_ms).and_then(find) {
        return Some(hit);
    }
    TABLE
        .iter()
        .filter(|(k, _)| key.starts_with(*k))
        .max_by_key(|(k, _)| k.len())
        .map(|(k, r)| (*k, r))
}

fn resolve(model: &str, at_ms: i64) -> Option<&'static Rate> {
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

pub fn cost_for(model: &str, t: &Tokens, at_ms: i64) -> Option<f64> {
    let r = resolve(model, at_ms)?;
    let context = t.input + t.cache_read + t.cache_write_5m + t.cache_write_1h;
    let (mi, mo, mc) = match r.long_context {
        Some(lc) if context > lc.threshold => (lc.input, lc.output, lc.cache_read),
        _ => (1.0, 1.0, 1.0),
    };
    let usd = (t.input as f64 * r.input * mi
        + t.output as f64 * r.output * mo
        + t.cache_write_5m as f64 * r.cache_write_5m * mi
        + t.cache_write_1h as f64 * r.cache_write_1h * mi
        + t.cache_read as f64 * r.cache_read * mc)
        / 1_000_000.0;
    Some(usd)
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
        let cost = cost_for(model, t, at_ms).unwrap_or_else(|| panic!("{model} should be priced"));
        (cost * 1e6).round() / 1e6
    }

    #[test]
    fn claude_cache_writes_use_tier_multipliers() {
        // 5-minute writes at 1.25× input, 1-hour at 2×: asymmetric so a swap
        // of the two fields is visible.
        let t = Tokens {
            cache_write_5m: 1_000_000,
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
            input: OPENAI_LONG_CONTEXT.threshold,
            ..Default::default()
        };
        assert_eq!(usd("gpt-6-astra", &boundary, 0), 2.72);
        // Haiku 5.5's tier counts cache writes toward the prompt and scales
        // them too, which models.dev's snapshot doesn't check.
        let writes = |n| Tokens {
            cache_write_5m: n,
            ..Default::default()
        };
        assert_eq!(usd("claude-haiku-5-5", &writes(100_000), 0), 0.0125);
        assert_eq!(usd("claude-haiku-5-5", &writes(200_000), 0), 0.125);
    }

    #[test]
    fn dated_names_match_the_longest_key() {
        // Also a prefix of `claude-opus-5`, which bills more.
        let t = short();
        assert_eq!(
            usd("claude-opus-5-5-20261001", &t, 0),
            usd("claude-opus-5-5", &t, 0)
        );
    }

    #[test]
    fn fast_resolution() {
        let t = short();
        // Dated and aliased names find the premium sibling of their resolved
        // base key instead of prefix-matching the standard rate.
        assert_eq!(
            usd("claude-opus-5-20260301-fast", &t, 0),
            usd("claude-opus-5-fast", &t, 0)
        );
        assert_eq!(usd("gpt-6-fast", &t, 0), usd("gpt-6-astra-fast", &t, 0));
        let at = AUTO_REVIEW_LUNA_AT_MS;
        assert_eq!(
            usd("codex-auto-review-fast", &t, at),
            usd("gpt-5.6-luna-fast", &t, at)
        );
        // A base with no premium key keeps its own rate rather than $0.
        assert_eq!(usd("gpt-5.4-nano-fast", &t, 0), usd("gpt-5.4-nano", &t, 0));
    }

    #[test]
    fn auto_review_switches_to_luna_on_cutover() {
        let t = short();
        let at = AUTO_REVIEW_LUNA_AT_MS;
        assert_eq!(usd("codex-auto-review", &t, at - 1), usd("gpt-5.5", &t, 0));
        assert_eq!(usd("codex-auto-review", &t, at), usd("gpt-5.6-luna", &t, 0));
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
            let [provider, model, input, output, cache_read, cache_write_5m, lc_size, lc_input, lc_output, lc_cache_read] =
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
                "anthropic" => pairs.push(("cache write", r.cache_write_5m, num(cache_write_5m))),
                // Codex logs carry no cache-write tokens, so that rate is never billed.
                "openai" => {}
                _ => panic!("unexpected provider in: {line}"),
            }
            // models.dev publishes no long-context tier for Fast modes.
            if !model.ends_with("-fast") {
                match (r.long_context, num(lc_size)) {
                    (None, None) => {}
                    (Some(_), None) => {
                        drift.push(format!("{model}: long-context tier, models.dev has none"))
                    }
                    (None, Some(_)) => {
                        drift.push(format!("{model}: no long-context tier, models.dev has one"))
                    }
                    (Some(lc), size) => {
                        pairs.extend([
                            ("long-context threshold", lc.threshold as f64, size),
                            ("long-context input", r.input * lc.input, num(lc_input)),
                            ("long-context output", r.output * lc.output, num(lc_output)),
                            (
                                "long-context cache read",
                                r.cache_read * lc.cache_read,
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
