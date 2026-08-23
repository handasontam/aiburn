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

#[derive(Clone, Copy, Default)]
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

/// Match ccusage's fallback for older Codex events that have no explicit
/// service tier: use the top-level `service_tier` from config.toml, otherwise
/// Standard. A missing/unknown value is deliberately conservative.
fn configured_service_tier() -> ServiceTier {
    let base = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")));
    let Some(base) = base else {
        return ServiceTier::Standard;
    };
    let Ok(config) = fs::read_to_string(base.join("config.toml")) else {
        return ServiceTier::Standard;
    };

    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') || line.starts_with('#') {
            continue;
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
    let base = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")));
    match base {
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
/// snapshot. A lower value means the session reset its counters.
fn delta(cur: RawUsage, prev: Option<RawUsage>) -> RawUsage {
    match prev {
        None => cur,
        Some(p) if cur.input < p.input || cur.cached < p.cached || cur.output < p.output => cur,
        Some(p) => RawUsage {
            input: cur.input.saturating_sub(p.input),
            cached: cur.cached.saturating_sub(p.cached),
            output: cur.output.saturating_sub(p.output),
        },
    }
}

fn build_report(path: &Path, q: &Query) -> Report {
    let session_id = uuid_from_name(path).unwrap_or_else(|| file_stem(path));

    let mut model: Option<String> = None;
    let mut service_tier = configured_service_tier();
    let mut seen_turn_context = false;
    let mut forked = false;
    let mut first_token_second: Option<String> = None;
    let mut project = String::new();
    let mut prev_total: Option<RawUsage> = None;
    let mut report = Report::default();

    for_each_line(path, |line| {
        let is_turn = contains(line, b"\"turn_context\"");
        let is_tok = contains(line, b"\"token_count\"");
        let is_settings = contains(line, b"\"thread_settings_applied\"");
        let is_meta = contains(line, b"\"session_meta\"");
        if !is_turn && !is_tok && !is_settings && !is_meta {
            return;
        }
        let Some(e) = parse_line(line) else { return };

        if e.typ.as_deref() == Some("session_meta") {
            forked |= e.forked_from_id.is_some();
            return;
        }
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

        // Current forked rollouts begin with a parent-history replay whose
        // token events share the child's creation second. Keep the final
        // inherited cumulative snapshot as the child's baseline.
        if forked {
            let second = ts.get(..19).unwrap_or(&ts).to_string();
            match &first_token_second {
                None => first_token_second = Some(second),
                Some(first) if first == &second => {
                    if let Some(total) = e.total {
                        prev_total = Some(total);
                    }
                    return;
                }
                Some(_) => {}
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
        let Some((ts_ms, date, month)) = parse_ts(&ts) else {
            return;
        };

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

    deduped
        .into_par_iter()
        .map(|f| build_report(f, q))
        .reduce(Report::default, Report::merged)
}

pub fn files() -> Vec<PathBuf> {
    codex_dirs().iter().flat_map(|d| find_jsonl(d)).collect()
}
