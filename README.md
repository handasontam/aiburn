# aitally

Fast, local usage & cost reporting for **Claude Code** and **Codex** — in one table.

A deliberately small alternative to [ccusage](https://github.com/ccusage/ccusage),
scoped to just the two agents I use. It reads your local session logs, prices
them offline, and prints per-day / per-month / per-session cost. Scanning ~1.5 GB
of logs takes **well under a second** — no network, no cache, no build step.

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

Scanned 770 files in 584ms
```

## Install

Requires [Bun](https://bun.sh). From the repo:

```sh
bun install      # dev-only: @types/bun for the optional typecheck
bun link         # registers the global `aitally` command
```

Then run `aitally` from anywhere.

> `bunx aitally` (no clone) works only once the package is published to npm.
> Until then, `bun link` is the local equivalent.

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

Pricing is a small curated table in [`src/pricing.ts`](src/pricing.ts), taken
from LiteLLM / models.dev (July 2026). Costs are always recomputed from tokens —
the logged `costUSD` is ignored so both agents are priced consistently.

- Claude cache creation is split into 5-minute (1.25× input) and 1-hour
  (2× input) tiers; cache reads are 0.1× input.
- Codex `input_tokens` includes cached tokens, so the cached portion is billed
  at the cache-read rate and the remainder at the input rate. `output_tokens`
  already includes reasoning tokens.

Claude totals match ccusage to within ~0.3%. Codex is approximate (~1–2%):
Codex logs carry no authoritative cost, and standard pricing is assumed.

To add or correct a model, edit the `RATES` table in `src/pricing.ts`.

## Why it's fast

- Files are scanned as raw bytes; only the handful of lines containing token
  usage are decoded and `JSON.parse`d (native `Buffer.indexOf`, no per-char loop).
- Files are read concurrently across CPU cores.
- Pricing is embedded — no network request.

## License

MIT
