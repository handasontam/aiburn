mod agg;
mod claude;
mod codex;
mod json;
mod model;
mod pricing;
mod time;
mod util;

use std::io::{IsTerminal, Write};
use std::time::Instant;

use agg::{Command, Footer, Group, Query, Report};
use model::Agent;
use util::{commas, cost, render_table, Paint};

const HELP: &str = "aitally — fast Claude Code + Codex usage & cost

Usage:
  aitally [command] [options]

Commands:
  daily      Per-day usage and cost (default)
  monthly    Per-month usage and cost
  session    Per-session usage and cost

Options:
  --since <YYYY-MM-DD>   Only include usage on/after this date
  --until <YYYY-MM-DD>   Only include usage on/before this date
  --claude               Only Claude Code
  --codex                Only Codex
  --all                  Show every session (session view; default: top 25)
  --json                 Emit JSON instead of a table
  -h, --help             Show this help
  -v, --version          Show version";

struct Options {
    command: Command,
    json: bool,
    all: bool,
    agent: Option<Agent>,
    since: Option<String>,
    until: Option<String>,
}

fn parse_args() -> Result<Options, i32> {
    let mut o = Options {
        command: Command::Daily,
        json: false,
        all: false,
        agent: None,
        since: None,
        until: None,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "daily" => o.command = Command::Daily,
            "monthly" => o.command = Command::Monthly,
            "session" => o.command = Command::Session,
            "--json" => o.json = true,
            "--all" => o.all = true,
            "--claude" => o.agent = Some(Agent::Claude),
            "--codex" => o.agent = Some(Agent::Codex),
            "--agent" => {
                i += 1;
                o.agent = match args.get(i).map(String::as_str) {
                    Some("claude") => Some(Agent::Claude),
                    Some("codex") => Some(Agent::Codex),
                    _ => {
                        eprintln!("aitally: --agent expects claude|codex");
                        return Err(1);
                    }
                };
            }
            "--since" => {
                i += 1;
                o.since = args.get(i).cloned();
            }
            "--until" => {
                i += 1;
                o.until = args.get(i).cloned();
            }
            "-h" | "--help" => {
                println!("{HELP}");
                return Err(0);
            }
            "-v" | "--version" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Err(0);
            }
            _ => {
                eprintln!("aitally: unknown argument \"{a}\"\n\n{HELP}");
                return Err(1);
            }
        }
        i += 1;
    }
    Ok(o)
}

fn short_model(m: &str) -> String {
    m.strip_prefix("claude-").unwrap_or(m).to_string()
}

fn dash(v: f64) -> String {
    if v == 0.0 {
        "–".to_string()
    } else {
        cost(v)
    }
}

fn render_period(groups: &[&Group], label: &str, paint: &Paint) -> String {
    let rows: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            vec![
                g.key.clone(),
                commas(g.input),
                commas(g.output),
                commas(g.cache),
                dash(g.claude_cost),
                dash(g.codex_cost),
                cost(g.total()),
            ]
        })
        .collect();
    let (mut ti, mut to, mut tc, mut tcl, mut tco) = (0u64, 0u64, 0u64, 0.0, 0.0);
    for g in groups {
        ti += g.input;
        to += g.output;
        tc += g.cache;
        tcl += g.claude_cost;
        tco += g.codex_cost;
    }
    let total = vec![
        "TOTAL".to_string(),
        commas(ti),
        commas(to),
        commas(tc),
        cost(tcl),
        cost(tco),
        cost(tcl + tco),
    ];
    render_table(
        &[label, "Input", "Output", "Cache", "Claude", "Codex", "Total"],
        &rows,
        &['l', 'r', 'r', 'r', 'r', 'r', 'r'],
        Some(&total),
        paint,
    )
}

fn render_session(groups: &[&Group], all: bool, paint: &Paint) -> (String, Option<String>) {
    let shown: &[&Group] = if all || groups.len() <= 25 {
        groups
    } else {
        &groups[..25]
    };
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|g| {
            let models = g
                .models
                .iter()
                .map(|m| short_model(m))
                .collect::<Vec<_>>()
                .join(", ");
            let models = if models.chars().count() > 26 {
                format!("{}…", models.chars().take(25).collect::<String>())
            } else {
                models
            };
            let last = if g.last_ts > 0 {
                time::iso_from_ms(g.last_ts)[..10].to_string()
            } else {
                "–".to_string()
            };
            vec![
                last,
                g.agents.iter().next().copied().unwrap_or("").to_string(),
                if g.project.is_empty() {
                    "–".to_string()
                } else {
                    g.project.clone()
                },
                models,
                commas(g.input + g.output + g.cache),
                cost(g.total()),
            ]
        })
        .collect();
    let table = render_table(
        &["Last active", "Agent", "Project", "Model(s)", "Tokens", "Cost"],
        &rows,
        &['l', 'l', 'l', 'l', 'r', 'r'],
        None,
        paint,
    );
    let note = if all || groups.len() <= 25 {
        None
    } else {
        Some(format!(
            "Showing 25 of {} sessions (most recent). Use --all to list them all.",
            groups.len()
        ))
    };
    (table, note)
}

fn render_footer(f: &Footer, elapsed_ms: u128, file_count: usize, paint: &Paint) -> String {
    let mut lines = vec![
        String::new(),
        format!(
            "{}  {:>11}   {} tokens",
            paint.cyan("Claude"),
            cost(f.claude_cost),
            commas(f.claude_tokens)
        ),
        format!(
            "{}  {:>11}   {} tokens",
            paint.green("Codex "),
            cost(f.codex_cost),
            commas(f.codex_tokens)
        ),
        format!(
            "{}  {}",
            paint.bold("Total "),
            paint.bold(&format!("{:>11}", cost(f.claude_cost + f.codex_cost)))
        ),
    ];
    if f.unpriced_tokens > 0 {
        lines.push(paint.yellow(&format!(
            "\n! {} tokens had no known pricing ({}); excluded from cost.",
            commas(f.unpriced_tokens),
            f.unpriced_models.iter().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    lines.push(paint.dim(&format!("\nScanned {file_count} files in {elapsed_ms}ms")));
    lines.join("\n")
}

fn json_report(command: &str, groups: &[&Group]) -> String {
    let esc = json::escape;
    let arr = |v: &[String]| {
        v.iter()
            .map(|s| format!("\"{}\"", esc(s)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = String::new();
    out.push_str(&format!("{{\n  \"command\": \"{command}\",\n  \"groups\": [\n"));
    let items: Vec<String> = groups
        .iter()
        .map(|g| {
            let agents: Vec<String> = g.agents.iter().map(|s| s.to_string()).collect();
            let models: Vec<String> = g.models.iter().cloned().collect();
            let last = match g.last_ts {
                0 => "null".to_string(),
                ms => format!("\"{}\"", time::iso_from_ms(ms)),
            };
            format!(
                "    {{\n      \"key\": \"{}\",\n      \"agents\": [{}],\n      \"project\": \"{}\",\n      \"models\": [{}],\n      \"input\": {},\n      \"output\": {},\n      \"cache\": {},\n      \"claudeCost\": {:.6},\n      \"codexCost\": {:.6},\n      \"totalCost\": {:.6},\n      \"lastActive\": {}\n    }}",
                esc(&g.key),
                arr(&agents),
                esc(&g.project),
                arr(&models),
                g.input,
                g.output,
                g.cache,
                g.claude_cost,
                g.codex_cost,
                g.total(),
                last
            )
        })
        .collect();
    out.push_str(&items.join(",\n"));
    out.push_str("\n  ]\n}");
    out
}

fn run() -> i32 {
    let o = match parse_args() {
        Ok(o) => o,
        Err(code) => return code,
    };
    let query = Query {
        command: o.command,
        agent: o.agent,
        since: o.since.clone(),
        until: o.until.clone(),
    };

    let t0 = Instant::now();
    let claude_files = if o.agent == Some(Agent::Codex) {
        Vec::new()
    } else {
        claude::files()
    };
    let codex_files = if o.agent == Some(Agent::Claude) {
        Vec::new()
    } else {
        codex::files()
    };
    let file_count = claude_files.len() + codex_files.len();

    let (claude_report, codex_report) = rayon::join(
        || claude::load(&claude_files, &query),
        || codex::load(&codex_files, &query),
    );
    let report: Report = claude_report.merged(codex_report);
    let elapsed = t0.elapsed().as_millis();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if report.groups.is_empty() {
        let _ = writeln!(out, "No usage found.");
        return 0;
    }

    let groups = report.sorted_groups(o.command);

    if o.json {
        let command = match o.command {
            Command::Daily => "daily",
            Command::Monthly => "monthly",
            Command::Session => "session",
        };
        let _ = writeln!(out, "{}", json_report(command, &groups));
        return 0;
    }

    let paint = Paint::new(out.is_terminal() && std::env::var_os("NO_COLOR").is_none());
    let _ = writeln!(out, "{}", paint.dim("aitally · Claude Code + Codex usage\n"));
    match o.command {
        Command::Session => {
            let (table, note) = render_session(&groups, o.all, &paint);
            let _ = writeln!(out, "{table}");
            if let Some(n) = note {
                let _ = writeln!(out, "{}", paint.dim(&format!("\n{n}")));
            }
        }
        cmd => {
            let label = if cmd == Command::Monthly { "Month" } else { "Date" };
            let _ = writeln!(out, "{}", render_period(&groups, label, &paint));
        }
    }
    let _ = writeln!(out, "{}", render_footer(&report.footer, elapsed, file_count, &paint));
    0
}

fn main() {
    std::process::exit(run());
}
