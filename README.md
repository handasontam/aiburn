# aiburn

Fast, fully local usage & cost reporting for **Claude Code** and **Codex**, in
one table. See how much you're burning on your coding agents.

```
$ aiburn
aiburn · Claude Code + Codex usage

Date        Agent   Models       Input  Output  Cache    Cost
──────────  ──────  ───────────  ─────  ──────  ─────  ──────
2026-07-20  claude  opus-5       2.81K    222K  24.4M  $20.44
            codex   gpt-5.6-sol  64.0K   2.42K   500K   $0.64
2026-07-21  claude  opus-5         127   84.8K  22.3M  $17.27
──────────  ──────  ───────────  ─────  ──────  ─────  ──────
Date        Agent   Models       Input  Output  Cache    Cost
TOTAL                            66.9K    309K  47.2M  $38.35

Claude       $37.71   47.0M tokens
Codex         $0.64   567K tokens
Total        $38.35
```

Each raw model gets its own row, with repeated date and agent labels suppressed
so model-level comparisons stay readable.
Token counts are compact (`K`/`M`/`B`/`T`) and costs exact — pass `--exact` for
full counts, or `--json` for raw integers. JSON groups mirror the table: one
object per (date/month, agent, model), with the raw model name in `models`.

## Why aiburn

- **Fully local & private** — it only reads the log files and local Codex config
  your agents already write on your machine. No network calls or account;
  nothing about your usage ever leaves your computer.
- **One command, no install** — `uvx aiburn` on macOS or Linux.
- **Both agents, one view** — Claude Code and Codex side by side, by day, month,
  or session.
- **Fast at any size** — scans gigabytes of history in well under a second, with
  near-instant startup, so it stays snappy as your logs grow.
- **Accurate cost** — recomputed from token counts (including cache tiers), with
  date-aware model aliases and Codex Standard/Fast/Priority pricing.

## Run it

No install needed — [uv](https://docs.astral.sh/uv/) fetches a prebuilt binary
and runs it:

```sh
uvx aiburn                 # macOS / Linux, any arch
uvx aiburn monthly
```

Keep it as a permanent command:

```sh
uv tool install aiburn     # then just `aiburn`
```

Or build from source with a [Rust toolchain](https://rustup.rs):

```sh
cargo install --git https://github.com/handasontam/aiburn
```

## Usage

```
aiburn [command] [options]

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
  --exact                Full token counts instead of K/M/B/T
  --json                 Emit JSON instead of a table
  -h, --help             Show this help
  -v, --version          Show version
```

## What it reads

Only your local coding-agent logs — read in place, never sent anywhere:

- **Claude Code** — `~/.claude/projects` (also `~/.config/claude` and
  `$CLAUDE_CONFIG_DIR`)
- **Codex** — `~/.codex/sessions` and `~/.codex/archived_sessions`, plus the
  top-level `service_tier` in `~/.codex/config.toml` (or `$CODEX_HOME`)

## Cost accuracy

Costs are recomputed from token counts with a built-in, pinned public pricing
table, including Claude's 5-minute and 1-hour cache tiers and per-model cache
rates (Fable 5.1 reads cache at $0.25/MTok, not the usual 10% of input).
Claude Code figures match ccusage's to the cent; ccusage's headline total will
still be higher if it also finds OpenCode, Gemini, or other agent logs that
aiburn does not read. Codex logs carry no authoritative cost, so aiburn uses
public list pricing: explicit `priority`/`fast` events use the matching Fast
rate, missing tiers follow the top-level Codex config, and unknown tiers fall
back to Standard. A Codex request whose whole context (fresh plus cached input)
exceeds 272K tokens is billed entirely at OpenAI's long-context tier, as
ccusage does.

Two details worth knowing:

- **Fast mode** costs more per token. Fast/Priority usage appears as its own
  `-fast` row for both agents, priced at the per-model premium rate.
- **Codex replay history is skipped.** Forking or resuming a session copies the
  parent rollout's records into the new file; those replayed token events
  belong to the parent's own file and are not charged again.

## Acknowledgements

Inspired by [ccusage](https://github.com/ccusage/ccusage) by
[@ryoppippi](https://github.com/ryoppippi) and contributors — a great, broader
tool that covers many more agents. aiburn is a smaller reimplementation focused
on just Claude Code + Codex. ccusage is MIT licensed.

## License

MIT
