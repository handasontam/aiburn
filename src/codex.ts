import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import { costFor } from "./pricing.ts";
import type { UsageRow } from "./types.ts";
import { findJsonl, lineHas, mapPool, scanLines, toLocalDate } from "./util.ts";

/** Existing Codex session directories to scan. */
export function codexDirs(): string[] {
	const home = process.env.CODEX_HOME || join(homedir(), ".codex");
	return [join(home, "sessions"), join(home, "archived_sessions")].filter(existsSync);
}

const UUID = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i;
const TURN_CONTEXT = Buffer.from('"turn_context"');
const TOKEN_COUNT = Buffer.from('"token_count"');

interface RawUsage {
	input_tokens?: number;
	cached_input_tokens?: number;
	output_tokens?: number;
	reasoning_output_tokens?: number;
	total_tokens?: number;
}

function sub(cur: RawUsage, prev: RawUsage | null): RawUsage {
	if (!prev) return cur;
	const d = (a?: number, b?: number) => Math.max(0, (a || 0) - (b || 0));
	return {
		input_tokens: d(cur.input_tokens, prev.input_tokens),
		cached_input_tokens: d(cur.cached_input_tokens, prev.cached_input_tokens),
		output_tokens: d(cur.output_tokens, prev.output_tokens),
		reasoning_output_tokens: d(cur.reasoning_output_tokens, prev.reasoning_output_tokens),
		total_tokens: d(cur.total_tokens, prev.total_tokens),
	};
}

async function parseFile(path: string, seenSessions: Set<string>, sink: UsageRow[]): Promise<void> {
	// One session per rollout file; skip if the same session id reappears
	// (e.g. a live session that was later archived).
	const sid = basename(path).match(UUID)?.[1];
	if (sid) {
		if (seenSessions.has(sid)) return;
		seenSessions.add(sid);
	}

	let buf: Buffer;
	try {
		buf = await readFile(path);
	} catch {
		return;
	}

	let model: string | undefined;
	let project = "";
	let prevTotal: RawUsage | null = null;
	const sessionId = sid || basename(path, ".jsonl");

	scanLines(buf, (b, s, e) => {
		const isTurn = lineHas(b, s, e, TURN_CONTEXT);
		const isToken = lineHas(b, s, e, TOKEN_COUNT);
		if (!isTurn && !isToken) return;
		let obj: any;
		try {
			obj = JSON.parse(b.toString("utf8", s, e));
		} catch {
			return;
		}
		const p = obj.payload;
		if (obj.type === "turn_context" && p) {
			if (typeof p.model === "string") model = p.model;
			if (typeof p.cwd === "string") project = basename(p.cwd);
			return;
		}
		if (obj.type !== "event_msg" || p?.type !== "token_count") return;

		const info = p.info;
		if (!info) return;
		const total: RawUsage | undefined = info.total_token_usage;
		let last: RawUsage | undefined = info.last_token_usage;
		if (!last && total) last = sub(total, prevTotal);
		if (total) prevTotal = total;
		if (!last) return;

		const rawInput = last.input_tokens || 0;
		const cached = Math.min(last.cached_input_tokens || 0, rawInput);
		const output = last.output_tokens || 0;
		if (rawInput === 0 && cached === 0 && output === 0) return;

		const ts = Date.parse(obj.timestamp);
		if (Number.isNaN(ts)) return;

		const tokens = {
			input: rawInput - cached,
			output, // already includes reasoning_output_tokens
			cacheWrite: 0,
			cacheWrite1h: 0,
			cacheRead: cached,
		};
		const m = model || "unknown";
		const { usd, priced } = costFor(m, tokens);
		const date = toLocalDate(ts);
		sink.push({
			agent: "codex",
			timestamp: ts,
			date,
			month: date.slice(0, 7),
			sessionId,
			project,
			model: m,
			tokens,
			cost: usd,
			priced,
		});
	});
}

export async function loadCodex(limit: number): Promise<UsageRow[]> {
	const files = codexDirs().flatMap(findJsonl);
	const seen = new Set<string>();
	const rows: UsageRow[] = [];
	await mapPool(files, limit, (f) => parseFile(f, seen, rows));
	return rows;
}
