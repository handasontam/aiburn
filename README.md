# aitally

Fast, local usage & cost reporting for **Claude Code** and **Codex** — in one table.

A deliberately small alternative to [ccusage](https://github.com/ccusage/ccusage),
scoped to just the two agents I use. It reads your local session logs, prices
them offline, and prints per-day / per-month / per-session cost. A single
static binary with **sub-millisecond startup**; it **streams** the logs so
memory stays flat even when they grow to tens of GB.

```
$ aitally
aitally · Claude Code + Codex usage

Date            Input     Output          Cache     Claude    Codex      Total
──────────  ─────────  ─────────  ─────────────  ─────────  ───────  ─────────
2026-07-16     11,326    348,040     46,373,051     $72.79        –     $72.79
2026-07-17      2,665    139,378     26,644,318     $33.02        –     $33.02
2026-07-20      2,690     98,911      8,214,564      $8.03        –      $8.03
──────────  ─────────  ─────────  ─────────────  ─────────  ───────  ─────────
TOTAL       2,982,331  1,534,324    256,344,781    $249.52   $64.81    $314.33

Claude      $249.52   177,853,108 tokens
Codex        $64.81    83,008,328 tokens
Total       $314.33

Scanned 1166 files in 413ms
```

## Install

Requires a [Rust toolchain](https://rustup.rs). From the repo:

```sh
cargo install --path .     # builds and installs `aitally` to ~/.cargo/bin
```

Then run `aitally` from anywhere. (Or `cargo build --release` and copy
`target/release/aitally` onto your `PATH`.)

## Usage

```
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
  -v, --version          Show version
```

## What it reads

- **Claude Code** — `~/.claude/projects/**/*.jsonl` (and `~/.config/claude`,
  `$CLAUDE_CONFIG_DIR`). Assistant `message.usage` records, deduplicated per
  `(message id, request id)` keeping the final streamed token counts.
- **Codex** — `~/.codex/sessions/**` and `~/.codex/archived_sessions/**`
  (or `$CODEX_HOME`). Per-turn `token_count` deltas, with the model taken from
  the surrounding `turn_context`.

## How cost is computed

Pricing is a small curated table in [`src/pricing.rs`](src/pricing.rs), taken
from LiteLLM / models.dev (July 2026). Costs are always recomputed from tokens —
the logged `costUSD` is ignored so both agents are priced consistently.

- Claude cache creation is split into 5-minute (1.25× input) and 1-hour
  (2× input) tiers; cache reads are 0.1× input.
- Codex `input_tokens` includes cached tokens, so the cached portion is billed
  at the cache-read rate and the remainder at the input rate. `output_tokens`
  already includes reasoning tokens.

Claude totals match ccusage to within ~0.3%. Codex is approximate (~1–2%):
Codex logs carry no authoritative cost, and standard pricing is assumed.

To add or correct a model, edit the `table()` in `src/pricing.rs`.

## Why it's fast (and light)

- **Streaming reader** — files are read one line at a time with a reused
  buffer, so a 150 MB session file costs one line of memory, not 150 MB.
- **Incremental aggregation** — each usage record is folded into the running
  totals ([`src/agg.rs`](src/agg.rs)) as it is parsed and then dropped. Peak
  memory scales with the number of *groups* (days / months / sessions), not the
  number of events, so it stays flat as Codex logs grow into the tens of GB.
  Claude is deduplicated by `(message id, request id)` first — bounded by
  message count, not raw log size.
- **Field-scanning parser** — a tiny hand-rolled JSON scanner
  ([`src/json.rs`](src/json.rs)) reads only the few fields it needs and skips
  everything else (the large `content` blocks) without allocating.
- **Parallel** — files are parsed across all cores with `rayon` (Codex via a
  parallel reduce over per-file reports).
- **Embedded pricing** — no network request, no config.
- **No heavyweight deps** — `rayon` + `libc` only; dates via `libc::localtime_r`.

Measured on ~1.5 GB of logs / ~2.9 B tokens (~1,166 files): cold ~1.2 s,
warm ~0.25 s, ~80 MB peak RSS, sub-millisecond process startup, ~470 KB binary.

## License

MIT
