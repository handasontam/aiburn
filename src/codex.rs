use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::agg::{Query, Report};
use crate::json::{contains, P};
use crate::model::{Agent, Tokens, UsageRow};
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

fn parse_payload(p: &mut P, e: &mut Extract) {
    if !p.enter_obj() {
        return;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "type" => e.payload_type = p.str_opt(),
            "model" => e.model = p.str_opt(),
            "cwd" => e.cwd = p.str_opt(),
            "info" => parse_info(p, e),
            _ => p.skip(),
        }
    }
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

fn sub(cur: RawUsage, prev: Option<RawUsage>) -> RawUsage {
    match prev {
        None => cur,
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
    let mut project = String::new();
    let mut prev_total: Option<RawUsage> = None;
    let mut report = Report::default();

    for_each_line(path, |line| {
        let is_turn = contains(line, b"\"turn_context\"");
        let is_tok = contains(line, b"\"token_count\"");
        if !is_turn && !is_tok {
            return;
        }
        let Some(e) = parse_line(line) else { return };

        if e.typ.as_deref() == Some("turn_context") {
            if let Some(m) = e.model {
                model = Some(m);
            }
            if let Some(cw) = e.cwd {
                project = basename(&cw);
            }
            return;
        }
        if e.typ.as_deref() != Some("event_msg") || e.payload_type.as_deref() != Some("token_count") {
            return;
        }

        let total = e.total;
        let last = e.last.or_else(|| total.map(|t| sub(t, prev_total)));
        if let Some(t) = total {
            prev_total = Some(t);
        }
        let Some(last) = last else { return };

        let cached = last.cached.min(last.input);
        if last.input == 0 && cached == 0 && last.output == 0 {
            return;
        }
        let Some(ts) = e.timestamp else { return };
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
        let (cost, priced) = cost_for(&m, &tokens);
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
