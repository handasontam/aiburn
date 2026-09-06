//! Minimal, allocation-light JSON scanner. We only extract a handful of scalar
//! fields per line and `skip` everything else (including the huge `content`
//! arrays) without building a value tree, so memory stays bounded.

pub struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        P { b, i: 0 }
    }

    fn peek(&self) -> u8 {
        if self.i < self.b.len() {
            self.b[self.i]
        } else {
            0
        }
    }

    fn ws(&mut self) {
        while matches!(self.peek(), b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    /// If the next value is an object, consume `{` and return true; otherwise
    /// skip the value (e.g. `null`) and return false.
    pub fn enter_obj(&mut self) -> bool {
        self.ws();
        if self.peek() == b'{' {
            self.i += 1;
            true
        } else {
            self.skip();
            false
        }
    }

    /// Next `"key":` in the current object, positioned at its value. Returns
    /// None at the closing `}` (which it consumes).
    pub fn obj_next(&mut self) -> Option<String> {
        self.ws();
        match self.peek() {
            b',' => {
                self.i += 1;
                self.ws();
            }
            b'}' => {
                self.i += 1;
                return None;
            }
            _ => {}
        }
        self.ws();
        match self.peek() {
            b'}' => {
                self.i += 1;
                None
            }
            b'"' => {
                let k = self.string();
                self.ws();
                if self.peek() == b':' {
                    self.i += 1;
                }
                k
            }
            _ => None,
        }
    }

    /// Parse a string value; skip and return None if the value isn't a string.
    pub fn str_opt(&mut self) -> Option<String> {
        self.ws();
        if self.peek() == b'"' {
            self.string()
        } else {
            self.skip();
            None
        }
    }

    /// Parse an integer value; skip and return None otherwise.
    pub fn u64(&mut self) -> Option<u64> {
        self.ws();
        let c = self.peek();
        if c == b'"' {
            return self.string().and_then(|s| s.parse::<u64>().ok());
        }
        if !(c == b'-' || c.is_ascii_digit()) {
            self.skip();
            return None;
        }
        let start = self.i;
        if self.peek() == b'-' {
            self.i += 1;
        }
        while self.peek().is_ascii_digit() {
            self.i += 1;
        }
        let end = self.i;
        if self.peek() == b'.' {
            self.i += 1;
            while self.peek().is_ascii_digit() {
                self.i += 1;
            }
        }
        if matches!(self.peek(), b'e' | b'E') {
            self.i += 1;
            if matches!(self.peek(), b'+' | b'-') {
                self.i += 1;
            }
            while self.peek().is_ascii_digit() {
                self.i += 1;
            }
        }
        std::str::from_utf8(&self.b[start..end])
            .ok()?
            .parse::<u64>()
            .ok()
    }

    fn string(&mut self) -> Option<String> {
        if self.peek() != b'"' {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hx = self.b.get(self.i..self.i + 4)?;
                            self.i += 4;
                            let code =
                                u32::from_str_radix(std::str::from_utf8(hx).ok()?, 16).ok()?;
                            let cp = if (0xD800..=0xDBFF).contains(&code)
                                && self.b.get(self.i) == Some(&b'\\')
                                && self.b.get(self.i + 1) == Some(&b'u')
                            {
                                let hx2 = self.b.get(self.i + 2..self.i + 6)?;
                                let lo =
                                    u32::from_str_radix(std::str::from_utf8(hx2).ok()?, 16).ok()?;
                                self.i += 6;
                                0x10000 + ((code - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                code
                            };
                            if let Some(ch) = char::from_u32(cp) {
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                        }
                        _ => return None,
                    }
                }
                _ => out.push(c),
            }
        }
        None
    }

    fn skip_string_raw(&mut self) {
        if self.peek() != b'"' {
            return;
        }
        self.i += 1;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            if c == b'\\' {
                self.i += 1;
            } else if c == b'"' {
                break;
            }
        }
    }

    /// Consume one JSON value of any type, discarding it.
    pub fn skip(&mut self) {
        self.ws();
        match self.peek() {
            b'{' => {
                self.i += 1;
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                    return;
                }
                loop {
                    self.ws();
                    self.skip_string_raw();
                    self.ws();
                    if self.peek() == b':' {
                        self.i += 1;
                    }
                    self.skip();
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        _ => {
                            if self.peek() == b'}' {
                                self.i += 1;
                            }
                            return;
                        }
                    }
                }
            }
            b'[' => {
                self.i += 1;
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    return;
                }
                loop {
                    self.skip();
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        _ => {
                            if self.peek() == b']' {
                                self.i += 1;
                            }
                            return;
                        }
                    }
                }
            }
            b'"' => self.skip_string_raw(),
            _ => {
                while self.i < self.b.len()
                    && !matches!(
                        self.b[self.i],
                        b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r'
                    )
                {
                    self.i += 1;
                }
            }
        }
    }
}

/// Substring test (first-byte scan then compare); replaces memchr offline.
pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    let n = needle.len();
    if n == 0 {
        return true;
    }
    if hay.len() < n {
        return false;
    }
    let first = needle[0];
    let last_start = hay.len() - n;
    let mut i = 0;
    while i <= last_start {
        match hay[i..=last_start].iter().position(|&b| b == first) {
            Some(off) => {
                let j = i + off;
                if &hay[j..j + n] == needle {
                    return true;
                }
                i = j + 1;
            }
            None => break,
        }
    }
    false
}

/// Escape a string for JSON output.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_lands_at_the_end_of_nested_and_escaped_values() {
        // Both loaders rely on `skip` to step over `content` arrays; a scanner
        // that stops early inside one silently loses the fields after it.
        let line = br#"{"content":[{"text":"a \"}\" ] b","n":[1,[2,{}]]},null,"x\\"],"model":"m","usage":{"input_tokens":"123","output_tokens":4.0}}"#;
        let mut p = P::new(line);
        assert!(p.enter_obj());
        let mut model = None;
        let mut input = None;
        let mut output = None;
        while let Some(k) = p.obj_next() {
            match k.as_str() {
                "model" => model = p.str_opt(),
                "usage" => {
                    assert!(p.enter_obj());
                    while let Some(kk) = p.obj_next() {
                        match kk.as_str() {
                            "input_tokens" => input = p.u64(),
                            "output_tokens" => output = p.u64(),
                            _ => p.skip(),
                        }
                    }
                }
                _ => p.skip(),
            }
        }
        assert_eq!(model.as_deref(), Some("m"));
        // Quoted integers count; a float-shaped value keeps its integer part.
        assert_eq!((input, output), (Some(123), Some(4)));
    }

    #[test]
    fn contains_handles_false_starts_and_edges() {
        assert!(contains(b"aab", b"ab"));
        assert!(contains(b"xyzab", b"ab"));
        assert!(!contains(b"a", b"ab"));
        assert!(!contains(b"abx", b"aby"));
    }
}
