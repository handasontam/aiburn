use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::agg::{Query, Report};
use crate::json::{contains, P};
use crate::model::{Agent, ServiceTier, Tokens, UsageRow};
use crate::pricing::cost_for;
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
    forked_from_id: Option<String>,
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
            "service_tier" => e.service_tier = p.str_opt(),
            "forked_from_id" => e.forked_from_id = p.str_opt(),
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

/// Match ccusage's fallback for older Codex events that have no explicit
/// service tier: use the top-level `service_tier` from config.toml, otherwise
/// Standard. A missing/unknown value is deliberately conservative.
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
pub fn codex_dirs() -> Vec<PathBuf> {
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
            && tail.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()));
        if ok {
            return Some(tail.join("-"));
        }
    }
    None
}

/// Convert a cumulative token snapshot into the usage since the previous
/// snapshot. When every counter is lower the session reset and the new
/// snapshot counts whole; a dip in only some fields is treated as noise and
/// clamped to zero rather than re-counting the whole session.
fn delta(cur: RawUsage, prev: Option<RawUsage>) -> RawUsage {
    match prev {
        None => cur,
        Some(p) if cur.input < p.input && cur.cached < p.cached && cur.output < p.output => cur,
        Some(p) => RawUsage {
            input: cur.input.saturating_sub(p.input),
            cached: cur.cached.saturating_sub(p.cached),
            output: cur.output.saturating_sub(p.output),
        },
    }
}

/// Longest pause tolerated inside a forked rollout's replayed history. Codex
/// rewrites the replay to the fork instant and writes it as one dense burst
/// (tens of milliseconds), while the child's own first turn follows a real
/// pause of seconds; ccusage's fallback uses the same gap heuristic.
const REPLAY_BURST_GAP_MS: i64 = 1_000;

fn build_report(path: &Path, q: &Query, fallback_tier: ServiceTier) -> Report {
    let session_id = uuid_from_name(path).unwrap_or_else(|| file_stem(path));

    let mut model: Option<String> = None;
    let mut service_tier = fallback_tier;
    let mut seen_turn_context = false;
    let mut forked = false;
    let mut replay_last_ms: Option<i64> = None;
    let mut replay_done = false;
    let mut first_line = true;
    let mut project = String::new();
    let mut prev_total: Option<RawUsage> = None;
    let mut report = Report::default();

    for_each_line(path, |line| {
        // session_meta is the rollout's opening record; only there can the
        // fork marker appear, so no other line pays for that needle.
        if std::mem::take(&mut first_line) && contains(line, b"\"session_meta\"") {
            forked = parse_line(line).is_some_and(|e| e.forked_from_id.is_some());
            return;
        }
        if !contains(line, b"\"token_count\"")
            && !contains(line, b"\"turn_context\"")
            && !contains(line, b"\"thread_settings_applied\"")
        {
            return;
        }
        let Some(e) = parse_line(line) else { return };

        if e.typ.as_deref() == Some("turn_context") {
            seen_turn_context = true;
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
        if e.typ.as_deref() != Some("event_msg") || e.payload_type.as_deref() != Some("token_count") {
            return;
        }
        let Some(ts) = e.timestamp else { return };
        let Some((ts_ms, date, month)) = parse_ts(&ts) else {
            return;
        };

        // Forked rollouts open with a replay of the parent's history. Absorb
        // the burst — first event included — as the child's baseline; the
        // first gap longer than REPLAY_BURST_GAP_MS is the child's own turn.
        if forked && !replay_done {
            match replay_last_ms {
                Some(prev) if ts_ms - prev > REPLAY_BURST_GAP_MS => replay_done = true,
                _ => {
                    replay_last_ms = Some(ts_ms);
                    if let Some(total) = e.total {
                        prev_total = Some(total);
                    }
                    return;
                }
            }
        }

        // Multi-agent Codex sessions replay the parent session's cumulative
        // token events before their first turn_context. Those events belong
        // to the parent and must not be attributed to this rollout.
        if !seen_turn_context {
            if let Some(total) = e.total {
                prev_total = Some(total);
            }
            return;
        }

        // `total_token_usage` is cumulative. Using `last_token_usage` for
        // every event double-counts re-emitted snapshots, which are common in
        // long Codex sessions. Use the cumulative delta whenever available and
        // retain the older field only for legacy records without totals.
        let last = match e.total {
            Some(total) => {
                let last = delta(total, prev_total);
                prev_total = Some(total);
                last
            }
            None => e.last.unwrap_or_default(),
        };

        let cached = last.cached.min(last.input);
        if last.input == 0 && cached == 0 && last.output == 0 {
            return;
        }

        let tokens = Tokens {
            input: last.input - cached,
            output: last.output, // already includes reasoning_output_tokens
            cache_write: 0,
            cache_write_1h: 0,
            cache_read: cached,
        };
        let m = model.clone().unwrap_or_else(|| "unknown".to_string());
        let (cost, priced) = cost_for(&m, &tokens, ts_ms, service_tier);
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

    report
}

pub fn load(files: &[PathBuf], q: &Query) -> Report {
    // One session per rollout file; drop files whose session id already
    // appeared (e.g. a live session later archived) before aggregating, so
    // the parallel reduce is a pure sum with no double counting.
    let mut sorted: Vec<&PathBuf> = files.iter().collect();
    sorted.sort();
    let mut seen: HashSet<String> = HashSet::new();
    let deduped: Vec<&PathBuf> = sorted
        .into_iter()
        .filter(|f| {
            let sid = uuid_from_name(f).unwrap_or_else(|| file_stem(f));
            seen.insert(sid)
        })
        .collect();

    let fallback_tier = configured_service_tier();
    deduped
        .into_par_iter()
        .map(|f| build_report(f, q, fallback_tier))
        .reduce(Report::default, Report::merged)
}

pub fn files() -> Vec<PathBuf> {
    codex_dirs().iter().flat_map(|d| find_jsonl(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_tier_reads_top_level_key() {
        assert_eq!(service_tier_from_config("service_tier = \"fast\""), ServiceTier::Fast);
        assert_eq!(service_tier_from_config("service_tier = 'priority'"), ServiceTier::Fast);
        assert_eq!(service_tier_from_config("service_tier = \"default\""), ServiceTier::Standard);
        assert_eq!(service_tier_from_config("model = \"gpt-5.6\""), ServiceTier::Standard);
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
        RawUsage { input, cached, output }
    }

    #[test]
    fn delta_counts_full_reset_whole() {
        let prev = raw(6_892_162, 6_111_360, 49_657);
        let cur = raw(83_569, 0, 1_328);
        assert_eq!(delta(cur, Some(prev)), cur);
    }

    #[test]
    fn delta_clamps_partial_dip_instead_of_recounting() {
        let prev = raw(900_000, 800_000, 100_000);
        let cur = raw(1_000_000, 750_000, 120_000);
        assert_eq!(delta(cur, Some(prev)), raw(100_000, 0, 20_000));
    }
}
