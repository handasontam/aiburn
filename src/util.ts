import { readdirSync } from "node:fs";
import { join } from "node:path";

/** Recursively collect every *.jsonl file under `root` (missing dir → []). */
export function findJsonl(root: string): string[] {
	const out: string[] = [];
	const stack: string[] = [root];
	while (stack.length > 0) {
		const dir = stack.pop() as string;
		let entries;
		try {
			entries = readdirSync(dir, { withFileTypes: true });
		} catch {
			continue;
		}
		for (const e of entries) {
			const full = join(dir, e.name);
			if (e.isDirectory()) stack.push(full);
			else if (e.isFile() && e.name.endsWith(".jsonl")) out.push(full);
		}
	}
	return out;
}

/**
 * Invoke `onLine(buf, start, end)` for each newline-delimited slice of `buf`.
 * Works on raw bytes so a whole file never has to be decoded to a JS string —
 * only the handful of lines we care about get decoded downstream. `Buffer.indexOf`
 * for the newline byte is a native scan, orders of magnitude faster than a
 * per-character JS loop.
 */
export function scanLines(buf: Buffer, onLine: (buf: Buffer, start: number, end: number) => void): void {
	let start = 0;
	const n = buf.length;
	while (start < n) {
		let nl = buf.indexOf(10, start);
		if (nl === -1) nl = n;
		if (nl > start) onLine(buf, start, nl);
		start = nl + 1;
	}
}

/** Native substring test scoped to a single line (bounded by a view). */
export function lineHas(buf: Buffer, start: number, end: number, needle: Buffer): boolean {
	return buf.subarray(start, end).indexOf(needle) !== -1;
}

/** Run `fn` over `items` with at most `limit` in flight at once. */
export async function mapPool<T>(
	items: T[],
	limit: number,
	fn: (item: T) => Promise<void>,
): Promise<void> {
	let i = 0;
	const worker = async () => {
		while (i < items.length) {
			const idx = i++;
			await fn(items[idx]);
		}
	};
	const n = Math.max(1, Math.min(limit, items.length));
	await Promise.all(Array.from({ length: n }, worker));
}

/** YYYY-MM-DD in local time. */
export function toLocalDate(ts: number): string {
	const d = new Date(ts);
	const y = d.getFullYear();
	const m = String(d.getMonth() + 1).padStart(2, "0");
	const day = String(d.getDate()).padStart(2, "0");
	return `${y}-${m}-${day}`;
}

// ---- formatting ----------------------------------------------------------

const useColor = Boolean(process.stdout.isTTY) && !process.env.NO_COLOR;
const wrap = (code: string) => (s: string) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s);
export const paint = {
	bold: wrap("1"),
	dim: wrap("2"),
	cyan: wrap("36"),
	green: wrap("32"),
	yellow: wrap("33"),
};

export const fmtInt = (n: number): string => Math.round(n).toLocaleString("en-US");

export const fmtCost = (n: number): string =>
	"$" + n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

type Align = "l" | "r";

/** Render a fixed-width table. Header and optional total row are emphasized. */
export function renderTable(
	headers: string[],
	rows: string[][],
	aligns: Align[],
	totalRow?: string[],
): string {
	const all = [headers, ...rows, ...(totalRow ? [totalRow] : [])];
	const widths = headers.map((_, i) => Math.max(...all.map((r) => (r[i] ?? "").length)));
	const gutter = "  ";
	const fmtRow = (r: string[]) =>
		r
			.map((c, i) => ((aligns[i] ?? "l") === "r" ? (c ?? "").padStart(widths[i]) : (c ?? "").padEnd(widths[i])))
			.join(gutter)
			.trimEnd();
	const rule = paint.dim(widths.map((w) => "─".repeat(w)).join(gutter));
	const lines = [paint.bold(fmtRow(headers)), rule];
	for (const r of rows) lines.push(fmtRow(r));
	if (totalRow) {
		lines.push(rule);
		lines.push(paint.bold(fmtRow(totalRow)));
	}
	return lines.join("\n");
}
