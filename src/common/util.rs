//! Error type and small string helpers (parsing, formatting, help text
//! wrapping, "did you mean" suggestions).

use std::fmt;

/// Failure of a command. `Usage` is an invalid command line and is reported
/// with exit code 2; `Runtime` is a failure while processing (I/O, decoding,
/// invalid image state) and is reported with exit code 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Usage(String),
    Runtime(String),
}

impl Error {
    pub fn usage(msg: impl Into<String>) -> Self {
        Error::Usage(msg.into())
    }

    pub fn runtime(msg: impl Into<String>) -> Self {
        Error::Runtime(msg.into())
    }

    pub fn message(&self) -> &str {
        match self {
            Error::Usage(m) | Error::Runtime(m) => m,
        }
    }

    pub fn is_usage(&self) -> bool {
        matches!(self, Error::Usage(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Returns `Err(Error::Usage(..))` from the enclosing function.
#[macro_export]
macro_rules! usage_err {
    ($($arg:tt)*) => { return Err($crate::common::Error::Usage(format!($($arg)*))) };
}

/// Returns `Err(Error::Runtime(..))` from the enclosing function.
#[macro_export]
macro_rules! runtime_err {
    ($($arg:tt)*) => { return Err($crate::common::Error::Runtime(format!($($arg)*))) };
}

/// ASCII-only lowercasing (option values and keys are ASCII).
pub fn to_lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Splits at every `sep`, keeping empty parts ("a,,b" gives 3 parts, ""
/// gives one empty part).
pub fn split(s: &str, sep: char) -> Vec<String> {
    s.split(sep).map(str::to_string).collect()
}

/// Strict number parsing: the whole string must be a finite decimal number
/// with a magnitude of at most 1e300. Leading whitespace, nan and inf are
/// rejected; they are never meaningful arguments here.
pub fn parse_double(s: &str) -> Option<f64> {
    let first = s.chars().next()?;
    if first.is_whitespace() {
        return None;
    }
    // Rust's float grammar is a subset of strtod's; additionally refuse the
    // spelled out specials early so "infinity" etc. never parse.
    let v: f64 = s.parse().ok()?;
    if !v.is_finite() || !(-1e300..=1e300).contains(&v) {
        return None;
    }
    Some(v)
}

/// Strict base-10 integer parsing: the whole string must be consumed.
pub fn parse_long(s: &str) -> Option<i64> {
    let first = s.chars().next()?;
    if first.is_whitespace() {
        return None;
    }
    s.parse().ok()
}

/// Formats a number like C++ `ostream << double` (printf `%g` with 6
/// significant digits): "0.5", "2", "1e-05", "1.5e+06".
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !v.is_finite() {
        return if v.is_nan() {
            "nan".into()
        } else if v > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    // Round to 6 significant digits first; the exponent of the rounded
    // value decides between fixed and scientific notation.
    let sci = format!("{:.5e}", v);
    let (mantissa, exp) = sci.split_once('e').expect("scientific format");
    let exp: i32 = exp.parse().expect("exponent");
    if !(-4..6).contains(&exp) {
        let m = strip_zeros(mantissa);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{m}e{sign}{:02}", exp.abs())
    } else {
        let decimals = (5 - exp).max(0) as usize;
        strip_zeros(&format!("{:.*}", decimals, v)).to_string()
    }
}

/// Removes trailing zeros (and a then trailing '.') after a decimal point.
fn strip_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// Word wrapping for help output. Continuation lines and paragraphs (from
/// explicit newlines in `text`) start with `indent` spaces; the first line
/// is assumed to already be at column `indent`.
pub fn wrap_text(text: &str, width: usize, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for (p, paragraph) in text.split('\n').enumerate() {
        if p > 0 {
            out.push('\n');
            out.push_str(&pad);
        }
        let mut col = indent;
        let mut first = true;
        for word in paragraph.split_whitespace() {
            let len = word.chars().count();
            if !first && col + 1 + len > width {
                out.push('\n');
                out.push_str(&pad);
                col = indent;
                first = true;
            }
            if !first {
                out.push(' ');
                col += 1;
            }
            out.push_str(word);
            col += len;
            first = false;
        }
    }
    out
}

/// Levenshtein distance (bytewise), used for "did you mean" suggestions.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(sub);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Returns the candidate closest to `word` (the first one on ties), or None
/// when nothing is reasonably close: the typo must be small relative to the
/// word length.
pub fn closest_match<S: AsRef<str>>(word: &str, candidates: &[S]) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = edit_distance(word, c.as_ref());
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c.as_ref(), d));
        }
    }
    let (name, dist) = best?;
    (dist <= (word.len() / 3).max(1)).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_ostream() {
        assert_eq!(format_number(0.01), "0.01");
        assert_eq!(format_number(100.0), "100");
        assert_eq!(format_number(1e-5), "1e-05");
        assert_eq!(format_number(0.5), "0.5");
        assert_eq!(format_number(65536.0), "65536");
        assert_eq!(format_number(1e6), "1e+06");
        assert_eq!(format_number(123456.7), "123457");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
    }

    #[test]
    fn strict_parsing() {
        assert_eq!(parse_double("1.5"), Some(1.5));
        assert_eq!(parse_double(".5"), Some(0.5));
        assert_eq!(parse_double(" 1"), None);
        assert_eq!(parse_double("1 "), None);
        assert_eq!(parse_double("nan"), None);
        assert_eq!(parse_double("inf"), None);
        assert_eq!(parse_double("1e400"), None);
        assert_eq!(parse_long("+7"), Some(7));
        assert_eq!(parse_long("7.0"), None);
    }

    #[test]
    fn wrapping_and_suggestions() {
        assert_eq!(wrap_text("aa bb cc", 7, 2), "aa bb\n  cc");
        assert_eq!(wrap_text("a\nb", 80, 2), "a\n  b");
        assert_eq!(closest_match("gravty", &["gravity", "x"]), Some("gravity".into()));
        assert_eq!(closest_match("zzzzzz", &["gravity"]), None);
    }
}
