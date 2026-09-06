use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Recursively collect every *.jsonl file under `root` (missing dir → []).
pub fn find_jsonl(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(ft) if ft.is_file() && path.extension().is_some_and(|e| e == "jsonl") => {
                    out.push(path);
                }
                _ => {}
            }
        }
    }
    out
}

/// Stream a file line by line, reusing one buffer so peak memory stays bounded
/// to a single line regardless of file size (files here reach ~150 MB).
pub fn for_each_line<F: FnMut(&[u8])>(path: &Path, mut f: F) {
    let file = match File::open(path) {
        Ok(x) => x,
        Err(_) => return,
    };
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut buf = Vec::with_capacity(8192);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => f(&buf),
            Err(_) => break,
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Last path segment of a slash-separated string.
pub fn basename(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

pub fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

// ---- formatting ----------------------------------------------------------

/// Group an integer with thousands separators.
pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Compact token count: `3.36B`, `50.4M`, `944K`, `56`. Precision tapers so
/// the string stays ~4 chars: 0 decimals ≥100, 1 decimal ≥10, else 2.
pub fn human(n: u64) -> String {
    let f = n as f64;
    let (v, suffix) = if f >= 1e12 {
        (f / 1e12, "T")
    } else if f >= 1e9 {
        (f / 1e9, "B")
    } else if f >= 1e6 {
        (f / 1e6, "M")
    } else if f >= 1e3 {
        (f / 1e3, "K")
    } else {
        return n.to_string();
    };
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    format!("{s}{suffix}")
}

/// Format a dollar amount as `$1,234.56`.
pub fn cost(f: f64) -> String {
    let neg = f < 0.0;
    let cents = (f.abs() * 100.0).round() as u64;
    format!(
        "{}${}.{:02}",
        if neg { "-" } else { "" },
        commas(cents / 100),
        cents % 100
    )
}

// ---- color ----------------------------------------------------------------

pub struct Paint {
    on: bool,
}

impl Paint {
    pub fn new(on: bool) -> Self {
        Paint { on }
    }
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    pub fn cyan(&self, s: &str) -> String {
        self.wrap("36", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.wrap("33", s)
    }
}

/// Render a fixed-width table. Header and optional total row are emphasized.
pub fn render_table(
    headers: &[&str],
    rows: &[Vec<String>],
    aligns: &[char],
    total: Option<&[String]>,
    paint: &Paint,
) -> String {
    let ncol = headers.len();
    let mut widths = vec![0usize; ncol];
    let mut consider = |r: &[String]| {
        for (i, cell) in r.iter().enumerate().take(ncol) {
            widths[i] = widths[i].max(cell.chars().count());
        }
    };
    let header_owned: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
    consider(&header_owned);
    for r in rows {
        consider(r);
    }
    if let Some(t) = total {
        consider(t);
    }

    let gutter = "  ";
    let fmt_row = |r: &[String]| -> String {
        let cells: Vec<String> = (0..ncol)
            .map(|i| {
                let cell = r.get(i).map(String::as_str).unwrap_or("");
                let pad = widths[i].saturating_sub(cell.chars().count());
                if aligns.get(i).copied().unwrap_or('l') == 'r' {
                    format!("{}{}", " ".repeat(pad), cell)
                } else {
                    format!("{}{}", cell, " ".repeat(pad))
                }
            })
            .collect();
        cells.join(gutter).trim_end().to_string()
    };
    let rule = paint.dim(
        &widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join(gutter),
    );

    let mut lines = vec![paint.bold(&fmt_row(&header_owned)), rule.clone()];
    for r in rows {
        lines.push(fmt_row(r));
    }
    if let Some(t) = total {
        lines.push(rule);
        // Repeat the header just above the total so column meanings are
        // visible where the terminal lands (bottom) without scrolling up.
        lines.push(paint.dim(&fmt_row(&header_owned)));
        lines.push(paint.bold(&fmt_row(t)));
    }
    lines.join("\n")
}
