export type Agent = "claude" | "codex";

export type Command = "daily" | "monthly" | "session";

export interface Tokens {
	/** Non-cached input tokens (billed at input rate). */
	input: number;
	/** Output tokens (includes reasoning tokens for Codex). */
	output: number;
	/** 5-minute cache-creation tokens (Claude; billed at 1.25× input). */
	cacheWrite: number;
	/** 1-hour cache-creation tokens (Claude; billed at 2× input). */
	cacheWrite1h: number;
	/** Cache-read tokens (billed at the discounted cache rate). */
	cacheRead: number;
}

export interface UsageRow {
	agent: Agent;
	/** Epoch milliseconds. */
	timestamp: number;
	/** YYYY-MM-DD in local time. */
	date: string;
	/** YYYY-MM in local time. */
	month: string;
	sessionId: string;
	project: string;
	model: string;
	tokens: Tokens;
	cost: number;
	/** False when no pricing was found for the model (cost is 0). */
	priced: boolean;
}

export interface Options {
	command: Command;
	json: boolean;
	since?: string; // YYYY-MM-DD
	until?: string; // YYYY-MM-DD
	agent?: Agent;
	all: boolean;
}
