import type { Tokens } from "./types.ts";

/** Rates in USD per 1,000,000 tokens. */
interface Rate {
	input: number;
	output: number;
	cacheWrite: number; // 5-minute cache creation
	cacheWrite1h: number; // 1-hour cache creation
	cacheRead: number;
}

const c = (input: number, output: number): Rate => ({
	// Anthropic: 5m cache write = 1.25× input, 1h cache write = 2× input,
	// cache read = 0.1× input.
	input,
	output,
	cacheWrite: input * 1.25,
	cacheWrite1h: input * 2,
	cacheRead: input * 0.1,
});

const o = (input: number, output: number, cacheRead: number): Rate => ({
	// OpenAI bills no separate cache-write; cached input has its own rate.
	input,
	output,
	cacheWrite: 0,
	cacheWrite1h: 0,
	cacheRead,
});

/**
 * Curated pricing for the models Claude Code and Codex actually emit.
 * Sourced from LiteLLM's model_prices_and_context_window.json (2026-07).
 * Keys are matched exactly, then by longest-prefix, so dated/regional
 * variants (e.g. "claude-opus-4-8-20260115") resolve to their base entry.
 */
const RATES: Record<string, Rate> = {
	// --- Anthropic / Claude Code ---
	"claude-opus-4-8": c(5, 25),
	"claude-opus-4-7": c(5, 25),
	"claude-opus-4-6": c(5, 25),
	"claude-opus-4-5": c(5, 25),
	"claude-opus-4-1": c(15, 75),
	"claude-opus-4": c(15, 75),
	"claude-fable-5": c(10, 50),
	"claude-sonnet-5": c(2, 10),
	"claude-sonnet-4-6": c(3, 15),
	"claude-sonnet-4-5": c(3, 15),
	"claude-sonnet-4": c(3, 15),
	"claude-3-5-sonnet": c(3, 15),
	"claude-haiku-4-5": c(1, 5),
	"claude-3-5-haiku": c(0.8, 4),
	"claude-3-opus": c(15, 75),
	"claude-3-haiku": c(0.25, 1.25),

	// --- OpenAI / Codex ---
	"gpt-5.6": o(5, 30, 0.5),
	"gpt-5.5": o(5, 30, 0.5),
	"gpt-5.4": o(2.5, 15, 0.25),
	"gpt-5.3-codex": o(1.75, 14, 0.175),
	"gpt-5.3": o(1.75, 14, 0.175),
	"gpt-5.2-codex": o(1.75, 14, 0.175),
	"gpt-5.2": o(1.75, 14, 0.175),
	"gpt-5.1-codex-mini": o(0.25, 2, 0.025),
	"gpt-5.1-codex-max": o(1.25, 10, 0.125),
	"gpt-5.1-codex": o(1.25, 10, 0.125),
	"gpt-5.1": o(1.25, 10, 0.125),
	"gpt-5-codex": o(1.25, 10, 0.125),
	"gpt-5-mini": o(0.25, 2, 0.025),
	"gpt-5-nano": o(0.05, 0.4, 0.005),
	"gpt-5": o(1.25, 10, 0.125),
	"codex-mini-latest": o(1.5, 6, 0.375),
	"codex-mini": o(1.5, 6, 0.375),
	"o3": o(2, 8, 0.5),
	"o4-mini": o(1.1, 4.4, 0.275),
};

/** Bare aliases and virtual model names → a concrete pricing key. */
const ALIASES: Record<string, string> = {
	opus: "claude-opus-4-8",
	sonnet: "claude-sonnet-5",
	haiku: "claude-haiku-4-5",
	// Codex review sub-agent runs on the current flagship model.
	"codex-auto-review": "gpt-5.5",
};

const rateCache = new Map<string, Rate | null>();

function resolveRate(model: string): Rate | null {
	const key = model.toLowerCase();
	const cached = rateCache.get(key);
	if (cached !== undefined) return cached;

	let rate: Rate | null = RATES[key] ?? null;
	if (!rate && ALIASES[key]) rate = RATES[ALIASES[key]] ?? null;
	if (!rate) {
		// longest matching prefix (handles dated/regional suffixes)
		let bestLen = 0;
		for (const k in RATES) {
			if (key.startsWith(k) && k.length > bestLen) {
				rate = RATES[k];
				bestLen = k.length;
			}
		}
	}
	rateCache.set(key, rate);
	return rate;
}

export function costFor(model: string, t: Tokens): { usd: number; priced: boolean } {
	const r = resolveRate(model);
	if (!r) return { usd: 0, priced: false };
	const usd =
		(t.input * r.input +
			t.output * r.output +
			t.cacheWrite * r.cacheWrite +
			t.cacheWrite1h * r.cacheWrite1h +
			t.cacheRead * r.cacheRead) /
		1_000_000;
	return { usd, priced: true };
}
