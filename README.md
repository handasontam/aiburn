# aiburn

Fast, local usage & cost reporting for **Claude Code** and **Codex** — in one
table. See how fast you're burning money on your coding agents.

A deliberately small alternative to [ccusage](https://github.com/ccusage/ccusage),
scoped to just the two agents I use. It reads your local session logs, prices
them offline, and prints per-day / per-month / per-session cost. A single
static binary with **sub-millisecond startup**; it **streams** the logs and
**aggregates incrementally**, so memory stays flat even as they grow to tens of GB.

```
$ aiburn
aiburn · Claude Code + Codex usage

Date        Models                 Input  Output  Cache   Claude  Codex    Total
──────────  ─────────────────────  ─────  ──────  ─────  ───────  ─────  ───────
2026-07-15  fable-5                1.54K    141K  12.9M   $33.88      –   $33.88
2026-07-16  fable-5, opus-4-8      11.3K    348K  46.4M   $72.79      –   $72.79
2026-07-17  fable-5, opus-4-8      2.67K    139K  26.6M   $33.02      –   $33.02
2026-07-20  opus-4-8, gpt-5.6-sol  66.8K    224K  24.9M   $20.44  $0.64   $21.08
──────────  ─────────────────────  ─────  ──────  ─────  ───────  ─────  ───────
Date        Models                 Input  Output  Cache   Claude  Codex    Total
TOTAL                              82.4K    916K   125M  $172.79  $0.64  $173.43

Claude      $172.79   126M tokens
Codex         $0.64   567K tokens
Total       $173.43

Scanned 1170 files in 288ms
```

Token counts are compact (`K`/`M`/`B`/`T`) and costs exact; the column header
repeats above the total so meanings stay on screen without scrolling. Use
`--exact` for full token counts and `--json` for raw integers.

## Run it

Once published to PyPI (see [Releasing](#releasing)), no install is needed —
[uv](https://docs.astral.sh/uv/) fetches the right prebuilt binary and runs it:

```sh
uvx aiburn                 # ephemeral run (macOS / Linux, any arch)
uvx aiburn monthly
```

To keep it around as a permanent command:

```sh
uv tool install aiburn     # then just `aiburn`
```

### From source

With a [Rust toolchain](https://rustup.rs):

```sh
cargo install --git https://github.com/handason/aiburn
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
- **Field-scanning parser** — a tiny hand-rolled JSON scanner
  ([`src/json.rs`](src/json.rs)) reads only the few fields it needs and skips
  everything else (the large `content` blocks) without allocating.
- **Parallel** — files are parsed across all cores with `rayon`.
- **No heavyweight deps** — `rayon` + `libc` only; dates via `libc::localtime_r`.

Measured on ~1.5 GB of logs / ~2.9 B tokens (~1,166 files): cold ~1.2 s,
warm ~0.25 s, ~80 MB peak RSS, sub-millisecond process startup, ~470 KB binary.

## Releasing

Publishing is what makes `uvx aiburn` work everywhere. Wheels (each carrying the
compiled binary for one platform) are built and pushed to PyPI by
[`.github/workflows/release.yml`](.github/workflows/release.yml) on every `v*`
tag, for macOS arm64/x86_64 and Linux x86_64/aarch64.

First-time setup:

1. Push this repo to `github.com/<you>/aiburn` and update the URL in
   [`pyproject.toml`](pyproject.toml).
2. On PyPI, add a **trusted publisher** for the project: owner `<you>`, repo
   `aiburn`, workflow `release.yml`, environment `pypi`. (No API token needed.)

Cut a release:

```sh
# bump version in Cargo.toml, then:
git tag v0.1.0 && git push --tags
```

The workflow builds all wheels and publishes them; `uvx aiburn` picks up the
new version automatically.

## Acknowledgements

Inspired by [ccusage](https://github.com/ccusage/ccusage) by
[@ryoppippi](https://github.com/ryoppippi) and contributors — a great, broader
tool that covers many more agents. aiburn is a smaller Rust reimplementation
focused on just Claude Code + Codex; the local log formats and the approach to
pricing were learned from ccusage. ccusage is MIT licensed.

## License

MIT
