#!/usr/bin/env bun
import { availableParallelism } from "node:os";
import { loadClaude } from "./claude.ts";
import { loadCodex } from "./codex.ts";
import type { Agent, Command, Options, UsageRow } from "./types.ts";
import { fmtCost, fmtInt, paint, renderTable } from "./util.ts";

const HELP = `aitally — fast Claude Code + Codex usage & cost

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
  -v, --version          Show version`;

function parseArgs(argv: string[]): Options | { help: true } | { version: true } {
	const opts: Options = { command: "daily", json: false, all: false };
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === "daily" || a === "monthly" || a === "session") opts.command = a as Command;
		else if (a === "--json") opts.json = true;
		else if (a === "--all") opts.all = true;
		else if (a === "--claude") opts.agent = "claude";
		else if (a === "--codex") opts.agent = "codex";
		else if (a === "--agent") opts.agent = argv[++i] as Agent;
		else if (a === "--since") opts.since = argv[++i];
		else if (a === "--until") opts.until = argv[++i];
		else if (a === "-h" || a === "--help") return { help: true };
		else if (a === "-v" || a === "--version") return { version: true };
		else {
			process.stderr.write(`aitally: unknown argument "${a}"\n\n${HELP}\n`);
			process.exit(1);
		}
	}
	return opts;
}

function inRange(row: UsageRow, o: Options): boolean {
	if (o.agent && row.agent !== o.agent) return false;
	if (o.since && row.date < o.since) return false;
	if (o.until && row.date > o.until) return false;
	return true;
}

interface Group {
	key: string;
	claudeCost: number;
	codexCost: number;
	input: number;
	output: number;
	cache: number;
	models: Set<string>;
	agents: Set<Agent>;
	projects: Set<string>;
	lastTs: number;
}

function newGroup(key: string): Group {
	return {
		key,
		claudeCost: 0,
		codexCost: 0,
		input: 0,
		output: 0,
		cache: 0,
		models: new Set(),
		agents: new Set(),
		projects: new Set(),
		lastTs: 0,
	};
}

function add(g: Group, r: UsageRow): void {
	if (r.agent === "claude") g.claudeCost += r.cost;
	else g.codexCost += r.cost;
	g.input += r.tokens.input;
	g.output += r.tokens.output;
	g.cache += r.tokens.cacheRead + r.tokens.cacheWrite + r.tokens.cacheWrite1h;
	g.models.add(r.model);
	g.agents.add(r.agent);
	if (r.project) g.projects.add(r.project);
	if (r.timestamp > g.lastTs) g.lastTs = r.timestamp;
}

function groupBy(rows: UsageRow[], keyOf: (r: UsageRow) => string): Group[] {
	const map = new Map<string, Group>();
	for (const r of rows) {
		let g = map.get(keyOf(r));
		if (!g) {
			g = newGroup(keyOf(r));
			map.set(g.key, g);
		}
		add(g, r);
	}
	return [...map.values()];
}

const shortModel = (m: string): string => m.replace(/^claude-/, "").replace(/^anthropic\./, "");
const cost = (g: Group) => g.claudeCost + g.codexCost;

function renderPeriodTable(groups: Group[], label: string): string {
	groups.sort((a, b) => a.key.localeCompare(b.key));
	const rows = groups.map((g) => [
		g.key,
		fmtInt(g.input),
		fmtInt(g.output),
		fmtInt(g.cache),
		g.claudeCost ? fmtCost(g.claudeCost) : "–",
		g.codexCost ? fmtCost(g.codexCost) : "–",
		fmtCost(cost(g)),
	]);
	const total = groups.reduce(
		(t, g) => {
			t.input += g.input;
			t.output += g.output;
			t.cache += g.cache;
			t.claude += g.claudeCost;
			t.codex += g.codexCost;
			return t;
		},
		{ input: 0, output: 0, cache: 0, claude: 0, codex: 0 },
	);
	const totalRow = [
		"TOTAL",
		fmtInt(total.input),
		fmtInt(total.output),
		fmtInt(total.cache),
		fmtCost(total.claude),
		fmtCost(total.codex),
		fmtCost(total.claude + total.codex),
	];
	return renderTable(
		[label, "Input", "Output", "Cache", "Claude", "Codex", "Total"],
		rows,
		["l", "r", "r", "r", "r", "r", "r"],
		totalRow,
	);
}

function renderSessionTable(groups: Group[], all: boolean): { table: string; note?: string } {
	groups.sort((a, b) => b.lastTs - a.lastTs);
	const shown = all ? groups : groups.slice(0, 25);
	const rows = shown.map((g) => {
		const models = [...g.models].map(shortModel).join(", ");
		return [
			g.lastTs ? new Date(g.lastTs).toISOString().slice(0, 10) : "–",
			[...g.agents][0] ?? "",
			[...g.projects][0] ?? "–",
			models.length > 26 ? models.slice(0, 25) + "…" : models,
			fmtInt(g.input + g.output + g.cache),
			fmtCost(cost(g)),
		];
	});
	const table = renderTable(
		["Last active", "Agent", "Project", "Model(s)", "Tokens", "Cost"],
		rows,
		["l", "l", "l", "l", "r", "r"],
	);
	const note =
		all || groups.length <= 25
			? undefined
			: `Showing 25 of ${groups.length} sessions (most recent). Use --all to list them all.`;
	return { table, note };
}

function renderFooter(rows: UsageRow[], elapsedMs: number, fileCount: number): string {
	let claude = 0;
	let codex = 0;
	let claudeTok = 0;
	let codexTok = 0;
	let unpricedTok = 0;
	const unpricedModels = new Set<string>();
	for (const r of rows) {
		const tok =
			r.tokens.input +
			r.tokens.output +
			r.tokens.cacheRead +
			r.tokens.cacheWrite +
			r.tokens.cacheWrite1h;
		if (r.agent === "claude") {
			claude += r.cost;
			claudeTok += tok;
		} else {
			codex += r.cost;
			codexTok += tok;
		}
		if (!r.priced) {
			unpricedTok += tok;
			unpricedModels.add(r.model);
		}
	}
	const lines = [
		"",
		`${paint.cyan("Claude")}  ${fmtCost(claude).padStart(11)}   ${fmtInt(claudeTok)} tokens`,
		`${paint.green("Codex ")}  ${fmtCost(codex).padStart(11)}   ${fmtInt(codexTok)} tokens`,
		`${paint.bold("Total ")}  ${paint.bold(fmtCost(claude + codex).padStart(11))}`,
	];
	if (unpricedTok > 0) {
		lines.push(
			paint.yellow(
				`\n! ${fmtInt(unpricedTok)} tokens had no known pricing (${[...unpricedModels].join(", ")}); excluded from cost.`,
			),
		);
	}
	lines.push(paint.dim(`\nScanned ${fileCount} files in ${Math.round(elapsedMs)}ms`));
	return lines.join("\n");
}

async function main(): Promise<void> {
	const parsed = parseArgs(process.argv.slice(2));
	if ("help" in parsed) {
		process.stdout.write(HELP + "\n");
		return;
	}
	if ("version" in parsed) {
		const pkg = await import("../package.json", { with: { type: "json" } });
		process.stdout.write((pkg.default?.version ?? "0.0.0") + "\n");
		return;
	}
	const o = parsed;

	const t0 = performance.now();
	const limit = Math.min(16, Math.max(1, availableParallelism()));
	const [claudeRows, codexRows] = await Promise.all([
		o.agent === "codex" ? Promise.resolve([]) : loadClaude(limit),
		o.agent === "claude" ? Promise.resolve([]) : loadCodex(limit),
	]);
	const all = [...claudeRows, ...codexRows].filter((r) => inRange(r, o));
	const elapsed = performance.now() - t0;

	if (all.length === 0) {
		process.stdout.write("No usage found.\n");
		return;
	}

	const keyOf =
		o.command === "monthly"
			? (r: UsageRow) => r.month
			: o.command === "session"
				? (r: UsageRow) => `${r.agent}:${r.sessionId}`
				: (r: UsageRow) => r.date;
	const groups = groupBy(all, keyOf);

	if (o.json) {
		const out = groups
			.map((g) => ({
				key: g.key,
				agents: [...g.agents],
				projects: [...g.projects],
				models: [...g.models],
				input: g.input,
				output: g.output,
				cache: g.cache,
				claudeCost: g.claudeCost,
				codexCost: g.codexCost,
				totalCost: cost(g),
				lastActive: g.lastTs ? new Date(g.lastTs).toISOString() : null,
			}))
			.sort((a, b) => (o.command === "session" ? 0 : a.key.localeCompare(b.key)));
		process.stdout.write(JSON.stringify({ command: o.command, groups: out }, null, 2) + "\n");
		return;
	}

	const fileCount =
		(o.agent === "codex" ? 0 : new Set(claudeRows.map((r) => r.sessionId)).size) +
		(o.agent === "claude" ? 0 : new Set(codexRows.map((r) => r.sessionId)).size);

	process.stdout.write(paint.dim("aitally · Claude Code + Codex usage\n\n"));
	if (o.command === "session") {
		const { table, note } = renderSessionTable(groups, o.all);
		process.stdout.write(table + "\n");
		if (note) process.stdout.write(paint.dim("\n" + note) + "\n");
	} else {
		process.stdout.write(renderPeriodTable(groups, o.command === "monthly" ? "Month" : "Date") + "\n");
	}
	process.stdout.write(renderFooter(all, elapsed, fileCount) + "\n");
}

main().catch((err) => {
	process.stderr.write(`aitally: ${err instanceof Error ? err.message : String(err)}\n`);
	process.exit(1);
});
