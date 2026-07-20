import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, join } from "node:path";
import { costFor } from "./pricing.ts";
import type { UsageRow } from "./types.ts";
import { findJsonl, lineHas, mapPool, scanLines, toLocalDate } from "./util.ts";

/** Existing Claude Code `projects` directories to scan. */
export function claudeDirs(): string[] {
	const candidates: string[] = [];
	const env = process.env.CLAUDE_CONFIG_DIR;
	if (env) for (const p of env.split(",")) candidates.push(join(p.trim(), "projects"));
	const home = homedir();
	candidates.push(join(home, ".claude", "projects"));
	candidates.push(join(home, ".config", "claude", "projects"));
	return [...new Set(candidates)].filter(existsSync);
}

/** Derive a friendly project name from Claude's `-Users-me-code-foo` folders. */
function projectFromFolder(folder: string): string {
	const parts = folder.split("-").filter(Boolean);
	return parts.length > 0 ? parts[parts.length - 1] : folder;
}

const INPUT_TOKENS = Buffer.from('"input_tokens"');

async function parseFile(
	path: string,
	keyed: Map<string, UsageRow>,
	keyless: UsageRow[],
): Promise<void> {
	let buf: Buffer;
	try {
		buf = await readFile(path);
	} catch {
		return;
	}
	const folderProject = projectFromFolder(basename(dirname(path)));

	scanLines(buf, (b, s, e) => {
		if (!lineHas(b, s, e, INPUT_TOKENS)) return;
		let obj: any;
		try {
			obj = JSON.parse(b.toString("utf8", s, e));
		} catch {
			return;
		}
		if (obj.type !== "assistant") return;
		const msg = obj.message;
		const u = msg?.usage;
		if (!u) return;

		const model: string = msg.model;
		if (!model || model === "<synthetic>") return;
		const ts = Date.parse(obj.timestamp);
		if (Number.isNaN(ts)) return;

		// Cache creation splits into 5-minute and 1-hour tiers (priced 1.25×
		// and 2× input). Fall back to the flat field as 5-minute when the
		// breakdown is absent.
		const cc = u.cache_creation;
		const tokens = {
			input: u.input_tokens || 0,
			output: u.output_tokens || 0,
			cacheWrite: cc ? cc.ephemeral_5m_input_tokens || 0 : u.cache_creation_input_tokens || 0,
			cacheWrite1h: cc ? cc.ephemeral_1h_input_tokens || 0 : 0,
			cacheRead: u.cache_read_input_tokens || 0,
		};
		const { usd, priced } = costFor(model, tokens);
		const date = toLocalDate(ts);
		const row: UsageRow = {
			agent: "claude",
			timestamp: ts,
			date,
			month: date.slice(0, 7),
			sessionId: obj.sessionId || basename(path, ".jsonl"),
			project: typeof obj.cwd === "string" ? basename(obj.cwd) : folderProject,
			model,
			tokens,
			cost: usd,
			priced,
		};

		// A streamed assistant message is logged repeatedly under the same
		// (message id, request id): input/cache stay constant while output
		// grows, so keep the record with the largest output (the final one).
		const id = msg.id;
		const req = obj.requestId;
		if (id && req) {
			const k = `${id}::${req}`;
			const prev = keyed.get(k);
			if (!prev || tokens.output > prev.tokens.output) keyed.set(k, row);
		} else {
			keyless.push(row);
		}
	});
}

export async function loadClaude(limit: number): Promise<UsageRow[]> {
	const files = claudeDirs().flatMap(findJsonl);
	const keyed = new Map<string, UsageRow>();
	const keyless: UsageRow[] = [];
	await mapPool(files, limit, (f) => parseFile(f, keyed, keyless));
	return [...keyed.values(), ...keyless];
}
