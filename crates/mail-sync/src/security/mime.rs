//! The little MIME the security code needs: splitting an entity into its
//! header and body, reading a Content-Type, cutting a multipart into its
//! parts byte for byte (signatures are over exact bytes), and writing
//! base64 bodies. Everything from outside is hostile: nothing here panics
//! or indexes past an end, and sizes are capped by the callers.

use base64::Engine as _;

/// An entity's header block (without the blank line) and its body.
#[must_use]
pub fn split(entity: &[u8]) -> (&[u8], &[u8]) {
    if entity.starts_with(b"\r\n") {
        return (&[], entity.get(2..).unwrap_or_default());
    }
    if entity.starts_with(b"\n") {
        return (&[], entity.get(1..).unwrap_or_default());
    }
    let crlf = find(entity, b"\r\n\r\n").map(|at| (at, 4));
    let lf = find(entity, b"\n\n").map(|at| (at, 2));
    let cut = match (crlf, lf) {
        (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
        (a, b) => a.or(b),
    };
    match cut {
        Some((at, len)) => (
            entity.get(..at).unwrap_or_default(),
            entity.get(at.saturating_add(len)..).unwrap_or_default(),
        ),
        None => (entity, &[]),
    }
}

/// Where `needle` first occurs in `haystack`.
#[must_use]
pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// One header field: its name as written and its unfolded value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The name, as written.
    pub name: String,
    /// The value, unfolded and trimmed.
    pub value: String,
    /// The field's bytes as they came, folding and line end included.
    pub raw: Vec<u8>,
}

/// The fields of a header block, in order.
#[must_use]
pub fn fields(header: &[u8]) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    for line in lines_with_ends(header) {
        let text = String::from_utf8_lossy(line);
        let starts_folded = line.first().is_some_and(|b| *b == b' ' || *b == b'\t');
        if starts_folded {
            if let Some(last) = out.last_mut() {
                last.value.push(' ');
                last.value.push_str(text.trim());
                last.raw.extend_from_slice(line);
            }
            continue;
        }
        let Some((name, value)) = text.split_once(':') else {
            continue;
        };
        out.push(Field {
            name: name.trim().to_owned(),
            value: value.trim().to_owned(),
            raw: line.to_vec(),
        });
    }
    // A last line without its end gets one, so fields can be put together.
    for field in &mut out {
        if !field.raw.ends_with(b"\n") {
            field.raw.extend_from_slice(b"\r\n");
        }
        field.value = field.value.trim().to_owned();
    }
    out
}

/// The lines of `bytes`, each with its line end.
fn lines_with_ends(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split_inclusive(|b| *b == b'\n')
}

/// The value of the first field called `name` (any case).
#[must_use]
pub fn header<'a>(fields: &'a [Field], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .map(|f| f.value.as_str())
}

/// A Content-Type: `type/subtype` lower case and its parameters (names
/// lower case).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentType {
    /// `type/subtype`, lower case; `text/plain` when there is none.
    pub value: String,
    params: Vec<(String, String)>,
}

impl ContentType {
    /// Read a Content-Type value.
    #[must_use]
    pub fn parse(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self {
                value: "text/plain".to_owned(),
                params: Vec::new(),
            };
        };
        let mut pieces = split_params(value).into_iter();
        let media = pieces
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let params = pieces
            .filter_map(|piece| {
                let (name, value) = piece.split_once('=')?;
                Some((name.trim().to_ascii_lowercase(), unquote(value.trim())))
            })
            .collect();
        Self {
            value: if media.is_empty() {
                "text/plain".to_owned()
            } else {
                media
            },
            params,
        }
    }

    /// A parameter's value.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// A parameter's value, trimmed and lower case.
    #[must_use]
    pub fn param_lower(&self, name: &str) -> Option<String> {
        self.param(name).map(|v| v.trim().to_ascii_lowercase())
    }
}

/// Split on `;` outside quotes.
fn split_params(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => {
                current.push(c);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ';' if !quoted => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// The parts of a multipart body, each exactly as it is between its
/// delimiter lines: the line end before a delimiter belongs to the
/// delimiter (RFC 2046 §5.1.1). At most `max` parts.
#[must_use]
pub fn parts<'a>(body: &'a [u8], boundary: &str, max: usize) -> Vec<&'a [u8]> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut out = Vec::new();
    // Where each delimiter line starts and where the line after it starts.
    let mut marks: Vec<(usize, usize, bool)> = Vec::new();
    let mut at = 0usize;
    while at < body.len() {
        let line_end = body
            .get(at..)
            .and_then(|rest| rest.iter().position(|b| *b == b'\n'))
            .map_or(body.len(), |p| at.saturating_add(p).saturating_add(1));
        let line = body.get(at..line_end).unwrap_or_default();
        if line.starts_with(delimiter) {
            let rest = line.get(delimiter.len()..).unwrap_or_default();
            let closing = rest.starts_with(b"--");
            let tail = if closing {
                rest.get(2..).unwrap_or_default()
            } else {
                rest
            };
            if tail
                .iter()
                .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            {
                marks.push((at, line_end, closing));
                if closing {
                    break;
                }
            }
        }
        at = line_end;
    }
    for pair in marks.windows(2) {
        let [(_, start, closing), (next, _, _)] = pair else {
            continue;
        };
        if *closing || out.len() >= max {
            break;
        }
        // Drop the line end that belongs to the next delimiter.
        let mut end = *next;
        if end > *start && body.get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
            if end > *start && body.get(end.saturating_sub(1)) == Some(&b'\r') {
                end = end.saturating_sub(1);
            }
        }
        out.push(body.get(*start..end.max(*start)).unwrap_or_default());
    }
    out
}

/// Lone LF line ends made CR LF: the canonical form signatures are over.
#[must_use]
pub fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len().saturating_add(bytes.len() / 32));
    let mut previous = 0u8;
    for &b in bytes {
        if b == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        previous = b;
    }
    out
}

/// A body decoded from its transfer encoding (base64 and quoted-printable;
/// anything else as it is).
#[must_use]
pub fn decode(encoding: Option<&str>, body: &[u8]) -> Vec<u8> {
    match encoding.map(|e| e.trim().to_ascii_lowercase()).as_deref() {
        Some("base64") => base64_lenient(body),
        Some("quoted-printable") => quoted_printable(body),
        _ => body.to_vec(),
    }
}

/// Base64 with line breaks and stray characters skipped.
#[must_use]
pub fn base64_lenient(text: &[u8]) -> Vec<u8> {
    let clean: Vec<u8> = text
        .iter()
        .copied()
        .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/'))
        .collect();
    let usable = clean.len().saturating_sub(clean.len() % 4);
    let remainder = clean.get(usable..).unwrap_or_default();
    let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
    let mut out = engine
        .decode(clean.get(..usable).unwrap_or_default())
        .unwrap_or_default();
    if remainder.len() >= 2 {
        out.extend(engine.decode(remainder).unwrap_or_default());
    }
    out
}

fn hex_value(b: u8) -> Option<u8> {
    char::from(b)
        .to_digit(16)
        .and_then(|d| u8::try_from(d).ok())
}

fn quoted_printable(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0usize;
    while let Some(&b) = body.get(i) {
        if b == b'=' {
            let one = body.get(i.saturating_add(1)).copied();
            let two = body.get(i.saturating_add(2)).copied();
            match (one, two) {
                (Some(b'\r'), Some(b'\n')) => i = i.saturating_add(3),
                (Some(b'\n'), _) => i = i.saturating_add(2),
                (Some(h), Some(l)) => {
                    if let (Some(h), Some(l)) = (hex_value(h), hex_value(l)) {
                        out.push(h.wrapping_mul(16).wrapping_add(l));
                        i = i.saturating_add(3);
                    } else {
                        out.push(b);
                        i = i.saturating_add(1);
                    }
                }
                _ => {
                    out.push(b);
                    i = i.saturating_add(1);
                }
            }
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    out
}

/// Base64 in lines of 76 characters, each ended with CR LF.
#[must_use]
pub fn base64_lines(bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut out = String::with_capacity(encoded.len().saturating_add(encoded.len() / 38));
    for chunk in encoded.as_bytes().chunks(76) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push_str("\r\n");
    }
    out
}

/// A new MIME boundary.
///
/// # Errors
/// When the system has no random numbers.
pub fn boundary() -> Result<String, crate::SyncError> {
    let mut bytes = [0u8; 18];
    getrandom::fill(&mut bytes)
        .map_err(|e| crate::SyncError::Invalid(format!("no randomness available: {e}")))?;
    Ok(format!(
        "=_mocky_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// Whether a field belongs to the content (it goes inside what is signed
/// or encrypted) rather than to the message.
#[must_use]
pub fn is_content_field(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("content-")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn splits_headers_from_the_body() {
        let (h, b) = split(b"A: 1\r\nB: 2\r\n\r\nbody\r\n");
        assert_eq!(h, b"A: 1\r\nB: 2");
        assert_eq!(b, b"body\r\n");
        let (h, b) = split(b"\r\nonly body");
        assert!(h.is_empty());
        assert_eq!(b, b"only body");
        let (h, b) = split(b"A: 1\nB: 2\n\nbody");
        assert_eq!(h, b"A: 1\nB: 2");
        assert_eq!(b, b"body");
    }

    #[test]
    fn reads_folded_fields_and_quoted_parameters() {
        let f = fields(b"Content-Type: multipart/signed;\r\n protocol=\"application/pkcs7-signature\";\r\n\tboundary=\"a;b\"\r\nX: y");
        assert_eq!(f.len(), 2);
        let ct = ContentType::parse(header(&f, "content-type"));
        assert_eq!(ct.value, "multipart/signed");
        assert_eq!(ct.param("protocol"), Some("application/pkcs7-signature"));
        assert_eq!(ct.param("boundary"), Some("a;b"));
        assert_eq!(f[1].raw, b"X: y\r\n");
        assert_eq!(ContentType::parse(None).value, "text/plain");
    }

    #[test]
    fn cuts_multiparts_exactly() {
        let body = b"preamble\r\n--b\r\nContent-Type: text/plain\r\n\r\nhi\r\n\r\n--b\r\nsecond\r\n--b--\r\nepilogue";
        let p = parts(body, "b", 8);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], b"Content-Type: text/plain\r\n\r\nhi\r\n");
        assert_eq!(p[1], b"second");
        // A line that only starts like the delimiter isn't one.
        let body = b"--b\r\nx\r\n--bb\r\ny\r\n--b--";
        assert_eq!(parts(body, "b", 8), vec![&b"x\r\n--bb\r\ny"[..]]);
    }

    #[test]
    fn decodes_and_encodes() {
        assert_eq!(base64_lenient(b"aGVs\r\nbG8=\r\n"), b"hello");
        assert_eq!(base64_lenient(b"aGVsbG8"), b"hello");
        assert_eq!(decode(Some("quoted-printable"), b"a=3Db=\r\nc"), b"a=bc");
        assert_eq!(crlf(b"a\nb\r\nc\n"), b"a\r\nb\r\nc\r\n");
        let lines = base64_lines(&[0u8; 100]);
        assert!(lines.lines().all(|l| l.len() <= 76));
        assert_eq!(base64_lenient(lines.as_bytes()), vec![0u8; 100]);
        assert!(boundary().unwrap().starts_with("=_mocky_"));
    }
}
