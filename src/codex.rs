use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::agg::{Query, Report};
use crate::json::{contains, P};
use crate::model::{Agent, ServiceTier, Tokens, UsageRow};
use crate::pricing::{cost_for, with_fast_suffix};
use crate::time::parse_ts;
use crate::util::{basename, file_stem, find_jsonl, for_each_line, home};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct RawUsage {
    input: u64,
    cached: u64,
    output: u64,
}

#[derive(Default)]
struct Extract {
    typ: Option<String>,
    timestamp: Option<String>,
    payload_type: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    service_tier: Option<String>,
    total: Option<RawUsage>,
    last: Option<RawUsage>,
}

fn parse_raw(p: &mut P) -> Option<RawUsage> {
    if !p.enter_obj() {
        return None;
    }
    let mut u = RawUsage::default();
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "input_tokens" => u.input = p.u64().unwrap_or(0),
            "cached_input_tokens" => u.cached = p.u64().unwrap_or(0),
            "output_tokens" => u.output = p.u64().unwrap_or(0),
            _ => p.skip(),
        }
    }
    Some(u)
}

fn parse_info(p: &mut P, e: &mut Extract) {
    if !p.enter_obj() {
        return;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "total_token_usage" => e.total = parse_raw(p),
            "last_token_usage" => e.last = parse_raw(p),
            _ => p.skip(),
        }
    }
}

fn parse_thread_settings(p: &mut P, e: &mut Extract) {
    if !p.enter_obj() {
        return;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "service_tier" => e.service_tier = p.str_opt(),
            _ => p.skip(),
        }
    }
}

fn parse_payload(p: &mut P, e: &mut Extract) {
    if !p.enter_obj() {
        return;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "type" => e.payload_type = p.str_opt(),
            "model" => e.model = p.str_opt(),
            "cwd" => e.cwd = p.str_opt(),
            "thread_settings" => parse_thread_settings(p, e),
            "info" => parse_info(p, e),
            _ => p.skip(),
        }
    }
}

/// Codex base directory: `$CODEX_HOME`, default `~/.codex`.
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")))
}

/// Fallback tier for events that never recorded one: the top-level
/// `service_tier` from config.toml, otherwise Standard. `[profiles.*]` tiers
/// are deliberately ignored (the log doesn't say which profile was active;
/// ccusage scans them too and can over-price inactive profiles). Note this
/// reads today's config, so re-tiering it re-prices old events that carry no
/// explicit tier of their own.
fn configured_service_tier() -> ServiceTier {
    codex_home()
        .and_then(|base| fs::read_to_string(base.join("config.toml")).ok())
        .map_or(ServiceTier::Standard, |c| service_tier_from_config(&c))
}

fn service_tier_from_config(config: &str) -> ServiceTier {
    for line in config.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            break; // keys below the first section header are not top-level
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "service_tier" {
            return ServiceTier::from_name(value.trim().trim_matches(['"', '\'']));
        }
    }
    ServiceTier::Standard
}

fn parse_line(bytes: &[u8]) -> Option<Extract> {
    let mut p = P::new(bytes);
    let mut e = Extract::default();
    if !p.enter_obj() {
        return None;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "type" => e.typ = p.str_opt(),
            "timestamp" => e.timestamp = p.str_opt(),
            "payload" => parse_payload(&mut p, &mut e),
            _ => p.skip(),
        }
    }
    Some(e)
}

/// Existing Codex session directories to scan.
fn codex_dirs() -> Vec<PathBuf> {
    match codex_home() {
        Some(b) => [b.join("sessions"), b.join("archived_sessions")]
            .into_iter()
            .filter(|p| p.is_dir())
            .collect(),
        None => Vec::new(),
    }
}

/// Extract the trailing UUID from a `rollout-<ts>-<uuid>.jsonl` filename.
fn uuid_from_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.split('-').collect();
    let n = parts.len();
    if n >= 5 {
        let tail = &parts[n - 5..];
        let ok = tail[0].len() == 8
            && tail[1].len() == 4
            && tail[2].len() == 4
            && tail[3].len() == 4
            && tail[4].len() == 12
            && tail
                .iter()
                .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()));
        if ok {
            return Some(tail.join("-"));
        }
    }
    None
}

/// One session per rollout file, identified by the filename's UUID.
fn session_id_of(path: &Path) -> String {
    uuid_from_name(path).unwrap_or_else(|| file_stem(path))
}

/// Saturating per-field difference between cumulative snapshots.
fn delta(cur: RawUsage, prev: Option<RawUsage>) -> RawUsage {
    match prev {
        None => cur,
        Some(p) => RawUsage {
            input: cur.input.saturating_sub(p.input),
            cached: cur.cached.saturating_sub(p.cached),
            output: cur.output.saturating_sub(p.output),
        },
    }
}

/// Usage recorded by one token_count event. Prefer the per-turn
/// `last_token_usage` — but only when the cumulative totals moved, because
/// long sessions re-emit unchanged snapshots whose `last` would double-count.
/// Records without `last` fall back to a saturating cumulative delta, so a
/// counter reset costs at most one skipped event rather than a re-counted
/// session. This matches ccusage.
fn event_usage(
    total: Option<RawUsage>,
    last: Option<RawUsage>,
    prev_total: &mut Option<RawUsage>,
) -> RawUsage {
    let moved = total.is_none_or(|t| *prev_total != Some(t));
    let usage = last
        .filter(|_| moved)
        .or_else(|| total.map(|t| delta(t, *prev_total)))
        .unwrap_or_default();
    if let Some(t) = total {
        *prev_total = Some(t);
    }
    usage
}

/// Longest pause tolerated inside a rollout's replayed history. Forking or
/// resuming a session rewrites the parent's records into the new file as one
/// dense burst (tens of milliseconds) stamped at the fork/resume instant,
/// while the session's own first turn follows a real pause of seconds;
/// ccusage's fallback uses the same gap cutoff.
const REPLAY_BURST_GAP_MS: i64 = 1_000;

fn build_report(path: &Path, session_id: String, q: &Query, fallback_tier: ServiceTier) -> Report {
    let mut model: Option<String> = None;
    let mut service_tier = fallback_tier;
    let mut seen_meta = false;
    // Some(ts) while absorbing a replay burst; the timestamp of the last
    // absorbed record.
    let mut replay_last_ms: Option<i64> = None;
    let mut project = String::new();
    let mut prev_total: Option<RawUsage> = None;
    let mut report = Report::default();

    let read = for_each_line(path, |line| {
        if !contains(line, b"\"token_count\"")
            && !contains(line, b"\"turn_context\"")
            && !contains(line, b"\"thread_settings_applied\"")
            && !contains(line, b"\"session_meta\"")
        {
            return;
        }
        let Some(e) = parse_line(line) else { return };

        // The first session_meta is the rollout's own opening record. Any
        // later one is a parent record copied in by a fork or resume: the
        // records that follow it — replayed history, token counts included —
        // belong to the parent's own rollout file and must not be charged
        // again here. Its timestamp is the rewrite instant, so it seeds the
        // burst window even when the replay carries no token events.
        if e.typ.as_deref() == Some("session_meta") {
            if seen_meta {
                if let Some((ms, _, _)) = e.timestamp.as_deref().and_then(parse_ts) {
                    replay_last_ms = Some(ms);
                }
            }
            seen_meta = true;
            return;
        }
        if e.typ.as_deref() == Some("turn_context") {
            if let Some(m) = e.model {
                model = Some(m);
            }
            if let Some(cw) = e.cwd {
                project = basename(&cw);
            }
            return;
        }
        if e.typ.as_deref() == Some("event_msg")
            && e.payload_type.as_deref() == Some("thread_settings_applied")
        {
            if let Some(tier) = e.service_tier {
                service_tier = ServiceTier::from_name(&tier);
            }
            return;
        }
        if e.typ.as_deref() != Some("event_msg") || e.payload_type.as_deref() != Some("token_count")
        {
            return;
        }
        let Some(ts) = e.timestamp else { return };
        let Some((ts_ms, date, month)) = parse_ts(&ts) else {
            return;
        };

        // Inside a replay burst: absorb the event as the parent's baseline.
        // The first event past the pause is the session's own turn and falls
        // through to be counted.
        if let Some(prev) = replay_last_ms {
            if ts_ms - prev > REPLAY_BURST_GAP_MS {
                replay_last_ms = None;
            } else {
                replay_last_ms = Some(ts_ms);
                if let Some(total) = e.total {
                    prev_total = Some(total);
                }
                return;
            }
        }

        let last = event_usage(e.total, e.last, &mut prev_total);
        let cached = last.cached.min(last.input);
        if last.input == 0 && last.output == 0 {
            return;
        }

        let tokens = Tokens {
            input: last.input - cached,
            output: last.output, // already includes reasoning_output_tokens
            cache_write: 0,
            cache_write_1h: 0,
            cache_read: cached,
        };
        let mut m = model.clone().unwrap_or_else(|| "unknown".to_string());
        if service_tier == ServiceTier::Fast {
            m = with_fast_suffix(m);
        }
        let (cost, priced) = cost_for(&m, &tokens, ts_ms);
        report.add(
            q,
            &UsageRow {
                agent: Agent::Codex,
                timestamp: ts_ms,
                date,
                month,
                session_id: session_id.clone(),
                project: project.clone(),
                model: m,
                tokens,
                cost,
                priced,
            },
        );
    });
    if read.is_err() {
        report.footer.unreadable_files += 1;
    }

    report
}

pub fn load(files: &[PathBuf], q: &Query) -> Report {
    if files.is_empty() {
        return Report::default();
    }
    // Drop files whose session id already appeared (e.g. a live session
    // later archived) before aggregating, so the parallel reduce is a pure
    // sum with no double counting.
    let mut sorted: Vec<&PathBuf> = files.iter().collect();
    sorted.sort();
    let mut seen: HashSet<String> = HashSet::new();
    let deduped: Vec<(&PathBuf, String)> = sorted
        .into_iter()
        .filter_map(|f| {
            let sid = session_id_of(f);
            seen.insert(sid.clone()).then_some((f, sid))
        })
        .collect();

    let fallback_tier = configured_service_tier();
    deduped
        .into_par_iter()
        .map(|(f, sid)| build_report(f, sid, q, fallback_tier))
        .reduce(Report::default, Report::merged)
}

pub fn files() -> Vec<PathBuf> {
    codex_dirs().iter().flat_map(|d| find_jsonl(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agg::Command;

    #[test]
    fn config_tier_reads_top_level_key() {
        assert_eq!(
            service_tier_from_config("service_tier = \"fast\""),
            ServiceTier::Fast
        );
        assert_eq!(
            service_tier_from_config("service_tier = 'priority'"),
            ServiceTier::Fast
        );
        assert_eq!(
            service_tier_from_config("service_tier = \"default\""),
            ServiceTier::Standard
        );
        assert_eq!(
            service_tier_from_config("model = \"gpt-5.6\""),
            ServiceTier::Standard
        );
    }

    #[test]
    fn config_tier_strips_inline_comments() {
        assert_eq!(
            service_tier_from_config("service_tier = \"fast\"  # premium"),
            ServiceTier::Fast
        );
    }

    #[test]
    fn config_tier_ignores_sectioned_keys() {
        assert_eq!(
            service_tier_from_config("[profiles.speedy]\nservice_tier = \"fast\""),
            ServiceTier::Standard
        );
    }

    fn raw(input: u64, cached: u64, output: u64) -> RawUsage {
        RawUsage {
            input,
            cached,
            output,
        }
    }

    #[test]
    fn event_usage_prefers_recorded_last_when_totals_move() {
        // A counter reset dips the totals, but the recorded per-turn usage
        // still counts and the baseline follows the new epoch.
        let mut prev = Some(raw(500_000, 0, 20_000));
        let got = event_usage(
            Some(raw(12_000, 0, 300)),
            Some(raw(12_000, 0, 300)),
            &mut prev,
        );
        assert_eq!(got, raw(12_000, 0, 300));
        assert_eq!(prev, Some(raw(12_000, 0, 300)));
    }

    #[test]
    fn event_usage_skips_reemitted_snapshots() {
        let mut prev = Some(raw(1_000, 0, 50));
        let got = event_usage(Some(raw(1_000, 0, 50)), Some(raw(1_000, 0, 50)), &mut prev);
        assert_eq!(got, RawUsage::default());
    }

    #[test]
    fn event_usage_falls_back_to_delta_without_last() {
        let mut prev = Some(raw(900, 100, 40));
        let got = event_usage(Some(raw(1_000, 100, 50)), None, &mut prev);
        assert_eq!(got, raw(100, 0, 10));
    }

    fn run(name: &str, lines: &[&str]) -> Report {
        let path = std::env::temp_dir().join(format!("aiburn-test-{name}.jsonl"));
        std::fs::write(&path, lines.join("\n")).unwrap();
        let q = Query {
            command: Command::Daily,
            since: None,
            until: None,
        };
        let report = build_report(&path, session_id_of(&path), &q, ServiceTier::Standard);
        let _ = std::fs::remove_file(&path);
        report
    }

    fn token_count(ts: &str, total: (u64, u64, u64), last: (u64, u64, u64)) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{},"cached_input_tokens":{},"output_tokens":{}}},"last_token_usage":{{"input_tokens":{},"cached_input_tokens":{},"output_tokens":{}}}}}}}}}"#,
            total.0, total.1, total.2, last.0, last.1, last.2
        )
    }

    #[test]
    fn replayed_history_is_absorbed() {
        // Fork/resume shape: own session_meta, then the parent's replayed
        // session_meta, turn_context, and a dense token burst — all stamped
        // at the rewrite instant — then the session's own turn after a real
        // pause. Only the own turn may be counted.
        let burst1 = token_count("2026-01-01T00:00:10.002Z", (1_000, 0, 50), (1_000, 0, 50));
        let burst2 = token_count("2026-01-01T00:00:10.003Z", (3_000, 0, 150), (2_000, 0, 100));
        let own = token_count("2026-01-01T00:00:20.000Z", (3_400, 0, 170), (400, 0, 20));
        let report = run(
            "replay",
            &[
                r#"{"timestamp":"2026-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"child"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:10.000Z","type":"session_meta","payload":{"id":"parent"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:10.001Z","type":"turn_context","payload":{"model":"gpt-5.2","cwd":"/p"}}"#,
                &burst1,
                &burst2,
                &own,
            ],
        );
        assert_eq!(report.footer.codex_tokens, 420);
    }

    #[test]
    fn unreadable_file_is_counted_not_ignored() {
        let path = std::env::temp_dir().join("aiburn-test-does-not-exist.jsonl");
        let q = Query {
            command: Command::Daily,
            since: None,
            until: None,
        };
        let report = build_report(&path, "x".to_string(), &q, ServiceTier::Standard);
        assert_eq!(report.footer.unreadable_files, 1);
    }

    #[test]
    fn plain_rollout_counts_every_turn() {
        // Two turns 100ms apart with no second session_meta: the burst gap
        // must not apply, or dense real turns would be swallowed as replay.
        // Codex's `input_tokens` includes the cached portion, so the second
        // turn's 500 cached tokens move from input to cache, not both.
        let t1 = token_count("2026-01-01T00:00:10.000Z", (1_000, 0, 50), (1_000, 0, 50));
        let t2 = token_count(
            "2026-01-01T00:00:10.100Z",
            (3_000, 500, 150),
            (2_000, 500, 100),
        );
        let report = run(
            "plain",
            &[
                r#"{"timestamp":"2026-01-01T00:00:00.000Z","type":"session_meta","payload":{"id":"solo"}}"#,
                r#"{"timestamp":"2026-01-01T00:00:05.000Z","type":"turn_context","payload":{"model":"gpt-5.2","cwd":"/p"}}"#,
                &t1,
                &t2,
            ],
        );
        assert_eq!(report.footer.codex_tokens, 3_150);
        let g = report.groups.values().next().unwrap();
        assert_eq!((g.input, g.cache, g.output), (2_500, 500, 150));
    }
}
