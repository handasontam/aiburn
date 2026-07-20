use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::json::{contains, P};
use crate::model::{Agent, Tokens, UsageRow};
use crate::pricing::cost_for;
use crate::time::parse_ts;
use crate::util::{basename, file_stem, find_jsonl, for_each_line, home};

#[derive(Default)]
struct Extract {
    typ: Option<String>,
    timestamp: Option<String>,
    request_id: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    msg_id: Option<String>,
    model: Option<String>,
    input: u64,
    output: u64,
    cache_creation_flat: u64,
    cache_5m: u64,
    cache_1h: u64,
    cache_read: u64,
    has_usage: bool,
}

fn parse_usage(p: &mut P, e: &mut Extract) {
    e.has_usage = true;
    if !p.enter_obj() {
        return;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "input_tokens" => e.input = p.u64().unwrap_or(0),
            "output_tokens" => e.output = p.u64().unwrap_or(0),
            "cache_creation_input_tokens" => e.cache_creation_flat = p.u64().unwrap_or(0),
            "cache_read_input_tokens" => e.cache_read = p.u64().unwrap_or(0),
            "cache_creation" => {
                if p.enter_obj() {
                    while let Some(kk) = p.obj_next() {
                        match kk.as_str() {
                            "ephemeral_5m_input_tokens" => e.cache_5m = p.u64().unwrap_or(0),
                            "ephemeral_1h_input_tokens" => e.cache_1h = p.u64().unwrap_or(0),
                            _ => p.skip(),
                        }
                    }
                }
            }
            _ => p.skip(),
        }
    }
}

fn parse_message(p: &mut P, e: &mut Extract) {
    if !p.enter_obj() {
        return;
    }
    while let Some(k) = p.obj_next() {
        match k.as_str() {
            "id" => e.msg_id = p.str_opt(),
            "model" => e.model = p.str_opt(),
            "usage" => parse_usage(p, e),
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
            "requestId" => e.request_id = p.str_opt(),
            "sessionId" => e.session_id = p.str_opt(),
            "cwd" => e.cwd = p.str_opt(),
            "message" => parse_message(&mut p, &mut e),
            _ => p.skip(),
        }
    }
    Some(e)
}

struct Parsed {
    keyed: Vec<(String, UsageRow)>,
    keyless: Vec<UsageRow>,
}

/// Existing Claude Code `projects` directories to scan.
pub fn claude_dirs() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(env) = std::env::var("CLAUDE_CONFIG_DIR") {
        for p in env.split(',') {
            candidates.push(Path::new(p.trim()).join("projects"));
        }
    }
    if let Some(h) = home() {
        candidates.push(h.join(".claude").join("projects"));
        candidates.push(h.join(".config").join("claude").join("projects"));
    }
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|p| seen.insert(p.clone()) && p.is_dir())
        .collect()
}

/// Derive a friendly project name from Claude's `-Users-me-code-foo` folders.
fn project_from_folder(path: &Path) -> String {
    let folder = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    folder
        .split('-')
        .filter(|s| !s.is_empty())
        .next_back()
        .unwrap_or(folder)
        .to_string()
}

fn parse_file(path: &Path) -> Parsed {
    let folder_project = project_from_folder(path);
    let stem = file_stem(path);
    let mut out = Parsed {
        keyed: Vec::new(),
        keyless: Vec::new(),
    };

    for_each_line(path, |line| {
        if !contains(line, b"\"input_tokens\"") {
            return;
        }
        let Some(e) = parse_line(line) else { return };
        if e.typ.as_deref() != Some("assistant") || !e.has_usage {
            return;
        }
        let model = match e.model {
            Some(m) if m != "<synthetic>" => m,
            _ => return,
        };
        let Some(ts) = e.timestamp else { return };
        let Some((ts_ms, date, month)) = parse_ts(&ts) else {
            return;
        };

        // Cache creation splits into 5-minute and 1-hour tiers (1.25× and 2×
        // input). Fall back to the flat field as 5-minute when absent.
        let (cw5, cw1h) = if e.cache_5m != 0 || e.cache_1h != 0 {
            (e.cache_5m, e.cache_1h)
        } else {
            (e.cache_creation_flat, 0)
        };
        let tokens = Tokens {
            input: e.input,
            output: e.output,
            cache_write: cw5,
            cache_write_1h: cw1h,
            cache_read: e.cache_read,
        };
        let (cost, priced) = cost_for(&model, &tokens);
        let row = UsageRow {
            agent: Agent::Claude,
            timestamp: ts_ms,
            date,
            month,
            session_id: e.session_id.unwrap_or_else(|| stem.clone()),
            project: e.cwd.as_deref().map(basename).unwrap_or_else(|| folder_project.clone()),
            model,
            tokens,
            cost,
            priced,
        };

        // A streamed assistant message is logged repeatedly under the same
        // (message id, request id): input/cache stay constant while output
        // grows, so keep the record with the largest output (the final one).
        match (e.msg_id, e.request_id) {
            (Some(id), Some(req)) => out.keyed.push((format!("{id}::{req}"), row)),
            _ => out.keyless.push(row),
        }
    });
    out
}

pub fn load(files: &[PathBuf]) -> Vec<UsageRow> {
    let parsed: Vec<Parsed> = files.par_iter().map(|f| parse_file(f)).collect();
    let mut keyed: HashMap<String, UsageRow> = HashMap::new();
    let mut rows: Vec<UsageRow> = Vec::new();
    for p in parsed {
        for (k, row) in p.keyed {
            match keyed.get(&k) {
                Some(prev) if prev.tokens.output >= row.tokens.output => {}
                _ => {
                    keyed.insert(k, row);
                }
            }
        }
        rows.extend(p.keyless);
    }
    rows.extend(keyed.into_values());
    rows
}

pub fn files() -> Vec<PathBuf> {
    claude_dirs().iter().flat_map(|d| find_jsonl(d)).collect()
}
