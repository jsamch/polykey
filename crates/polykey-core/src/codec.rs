//! String codec for the BCP formats: tags, hashes, canonical form, encoding and strict
//! parsing. Mirrors `reference/bcp_shares.py` (see `docs/DECISIONS.md`, entry 3, for the
//! few places where parsing is stricter than the reference).
//!
//! Parsing never panics on any input; every failure is a [`ParseError`].

use data_encoding::{Encoding, Specification};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::OnceLock;
use zeroize::{Zeroize, Zeroizing};

/// Length in bytes of the secret, and of every data field once decoded.
pub const DATA_LEN: usize = 32;

/// The four string tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tag {
    /// Unlocked share.
    Bcp1,
    /// Passcode-locked share.
    Bcp2,
    /// Unlocked master plate.
    Bcpk1,
    /// Passcode-locked master plate.
    Bcpk2,
}

impl Tag {
    /// All tags, in reference order.
    pub const ALL: [Tag; 4] = [Tag::Bcp1, Tag::Bcp2, Tag::Bcpk1, Tag::Bcpk2];

    /// The tag text as it appears in strings.
    pub fn as_str(self) -> &'static str {
        match self {
            Tag::Bcp1 => "BCP1",
            Tag::Bcp2 => "BCP2",
            Tag::Bcpk1 => "BCPK1",
            Tag::Bcpk2 => "BCPK2",
        }
    }

    /// Looks a tag up by its exact (uppercase) text.
    pub fn from_str_exact(s: &str) -> Option<Tag> {
        Tag::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Number of fields before the data field, including the tag itself.
    pub fn head(self) -> usize {
        match self {
            Tag::Bcp1 | Tag::Bcp2 => 5,
            Tag::Bcpk1 | Tag::Bcpk2 => 2,
        }
    }

    /// Number of fields after the data field (VER if locked, then CHECK).
    pub fn tail(self) -> usize {
        match self {
            Tag::Bcp1 | Tag::Bcpk1 => 1,
            Tag::Bcp2 | Tag::Bcpk2 => 2,
        }
    }

    /// True for BCP1 and BCP2.
    pub fn is_share(self) -> bool {
        matches!(self, Tag::Bcp1 | Tag::Bcp2)
    }

    /// True for BCP2 and BCPK2, whose data is XOR-locked with a passcode mask.
    pub fn is_locked(self) -> bool {
        matches!(self, Tag::Bcp2 | Tag::Bcpk2)
    }
}

/// Which kind of string a parser was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringKind {
    /// BCP1 or BCP2.
    Share,
    /// BCPK1 or BCPK2.
    Master,
}

/// Why a string was rejected. Display text equals the reference messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The tag is not one of the requested kind.
    NotRecognised { kind: StringKind },
    /// Wrong number of colon-separated fields for the tag.
    WrongFieldCount,
    /// CHECK does not match the text.
    ChecksumMismatch,
    /// The data field is not valid unpadded base32.
    MalformedData,
    /// The data field does not decode to 32 bytes.
    WrongLength,
    /// x, k or n is not a plain decimal number.
    MalformedShareFields,
    /// x, k or n is outside 2 <= k <= n <= 255, 1 <= x <= n.
    OutOfRange,
    /// A BCPK1 key does not hash to its set ID.
    SetIdMismatch,
}

impl ParseError {
    /// The stable snake_case id used in the golden vectors.
    pub fn category(&self) -> &'static str {
        match self {
            ParseError::NotRecognised { .. } => "not_recognised",
            ParseError::WrongFieldCount => "wrong_field_count",
            ParseError::ChecksumMismatch => "checksum_mismatch",
            ParseError::MalformedData => "malformed_data",
            ParseError::WrongLength => "wrong_length",
            ParseError::MalformedShareFields => "malformed_share_fields",
            ParseError::OutOfRange => "out_of_range",
            ParseError::SetIdMismatch => "set_id_mismatch",
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::NotRecognised {
                kind: StringKind::Share,
            } => "not a recognised share string",
            ParseError::NotRecognised {
                kind: StringKind::Master,
            } => "not a recognised master string",
            ParseError::WrongFieldCount => "wrong number of fields",
            ParseError::ChecksumMismatch => "checksum mismatch (typo or damaged plate)",
            ParseError::MalformedData => "malformed data field",
            ParseError::WrongLength => "data field has the wrong length",
            ParseError::MalformedShareFields => "malformed share fields",
            ParseError::OutOfRange => "share fields out of range",
            ParseError::SetIdMismatch => "key does not match its set ID",
        })
    }
}

impl std::error::Error for ParseError {}

/// A parsed share. `data` is locked for BCP2 and plain for BCP1; treat both as secret.
#[derive(Clone, PartialEq, Eq)]
pub struct ParsedShare {
    pub tag: Tag,
    pub x: u8,
    pub k: u8,
    pub n: u8,
    pub set_id: String,
    pub data: Zeroizing<[u8; DATA_LEN]>,
    pub ver: Option<String>,
}

impl fmt::Debug for ParsedShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedShare")
            .field("tag", &self.tag)
            .field("x", &self.x)
            .field("k", &self.k)
            .field("n", &self.n)
            .field("set_id", &self.set_id)
            .field("data", &"<redacted>")
            .field("ver", &self.ver)
            .finish()
    }
}

/// A parsed master plate. `data` is locked for BCPK2 and plain for BCPK1.
#[derive(Clone, PartialEq, Eq)]
pub struct ParsedMaster {
    pub tag: Tag,
    pub set_id: String,
    pub data: Zeroizing<[u8; DATA_LEN]>,
    pub ver: Option<String>,
}

impl fmt::Debug for ParsedMaster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedMaster")
            .field("tag", &self.tag)
            .field("set_id", &self.set_id)
            .field("data", &"<redacted>")
            .field("ver", &self.ver)
            .finish()
    }
}

// ------------------------------------------------------------------ hashes and base32

fn hex_upper(bytes: &[u8], chars: usize) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(chars);
    for i in 0..chars {
        let b = bytes[i / 2];
        let nib = if i % 2 == 0 { b >> 4 } else { b & 0x0F };
        out.push(DIGITS[nib as usize] as char);
    }
    out
}

fn base32() -> &'static Encoding {
    static ENC: OnceLock<Encoding> = OnceLock::new();
    ENC.get_or_init(|| {
        let mut spec = Specification::new();
        spec.symbols.push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567");
        // Stricter than the reference (DECISIONS.md entry 3): the unused low bits of the last
        // character must be zero, as every written string has them.
        spec.check_trailing_bits = true;
        spec.encoding()
            .expect("static base32 specification is valid")
    })
}

/// RFC 4648 base32, uppercase, padding stripped.
pub fn b32(data: &[u8]) -> String {
    base32().encode(data)
}

/// Decodes unpadded base32. Explicit `=` padding and non-zero trailing bits are rejected
/// (strict rule), as is any character outside `A-Z2-7`. The input is expected to be uppercase already.
pub fn unb32(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    base32().decode(text.as_bytes()).ok().map(Zeroizing::new)
}

/// SHA-256 of the concatenation of `parts`. The hasher's block buffer holds the input, which
/// can be the key or a plain share, and is not wiped by `sha2`, so the stack it used is
/// scrubbed afterwards.
fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    #[inline(never)]
    fn digest(parts: &[&[u8]]) -> [u8; 32] {
        let mut h = Sha256::new();
        for p in parts {
            h.update(p);
        }
        h.finalize().into()
    }
    let d = digest(parts);
    crate::wipe::scrub_stack();
    d
}

/// BCP1 set ID: first 8 uppercase hex characters of SHA-256(secret).
pub fn set_id(secret: &[u8]) -> String {
    hex_upper(&sha256(&[secret]), 8)
}

/// Verifier: first 3 uppercase hex characters of SHA-256("BCP2-verifier|" + secret).
pub fn verifier(secret: &[u8]) -> String {
    hex_upper(&sha256(&[b"BCP2-verifier|", secret]), 3)
}

/// CHECK: first 4 uppercase hex characters of SHA-256 of the body's UTF-8 bytes.
pub fn check(body: &str) -> String {
    hex_upper(&sha256(&[body.as_bytes()]), 4)
}

/// Splits `s` into groups of `size` characters joined by single spaces (`size` 0 acts as 1).
///
/// The result is built in one buffer sized up front, with no intermediate copies, because it
/// is also the passphrase reading aid.
pub fn group(s: &str, size: usize) -> String {
    let size = size.max(1);
    // At most one space per `size` characters, and a character is at least one byte.
    let mut out = String::with_capacity(s.len() + s.len() / size);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && i % size == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

// ------------------------------------------------------------------ canonical form

/// Whitespace as Python's `str.split()` sees it (Unicode White_Space plus U+001C..U+001F).
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Room for the upper-case form of a text of `len` bytes: no character's upper-case mapping
/// is more than three times its UTF-8 length (checked over every character in the tests).
/// Buffers sized this way never reallocate, so no stale partial copy of a plate string is left
/// in freed memory.
fn upper_capacity(len: usize) -> usize {
    len.saturating_mul(3)
}

/// Upper-cases and drops whitespace and dashes, in one pass and one buffer.
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(upper_capacity(text.len()));
    for u in text.chars().flat_map(char::to_uppercase) {
        if !is_py_space(u) && u != '-' {
            out.push(u);
        }
    }
    out
}

/// Normalises any accepted form to the colon form used for checksums. Only upper-cases and
/// removes spaces and dashes (or joins space-form tokens); typing slips are fixed in parsing.
///
/// The input may be a plain share or key, so intermediate text lives only in wiped or
/// pre-sized buffers. The caller owns the result and should hold it in a `Zeroizing`.
pub fn canonical(text: &str) -> String {
    if text.contains(':') {
        return clean(text);
    }
    // Same as `text.to_uppercase().replace('-', " ")`: no upper-case mapping yields a dash.
    let mut up = Zeroizing::new(String::with_capacity(upper_capacity(text.len())));
    for c in text.chars() {
        if c == '-' {
            up.push(' ');
        } else {
            up.extend(c.to_uppercase());
        }
    }
    let tokens: Vec<&str> = up.split(is_py_space).filter(|w| !w.is_empty()).collect();
    if let Some(tag) = tokens.first().and_then(|t| Tag::from_str_exact(t)) {
        let (h, tl) = (tag.head(), tag.tail());
        if tokens.len() > h + tl {
            // Each of the h + tl colons stands for at least one separator byte of `up`, so
            // the result fits in `up.len()` bytes.
            let mut out = String::with_capacity(up.len());
            for (i, t) in tokens[..h].iter().enumerate() {
                if i > 0 {
                    out.push(':');
                }
                out.push_str(t);
            }
            out.push(':');
            for t in &tokens[h..tokens.len() - tl] {
                out.push_str(t);
            }
            for t in &tokens[tokens.len() - tl..] {
                out.push(':');
                out.push_str(t);
            }
            return out;
        }
    }
    clean(text)
}

/// QR payload: the canonical text with spaces instead of colons.
pub fn qr_payload(canonical_text: &str) -> String {
    canonical_text.replace(':', " ")
}

/// Fields of a colon-form string split around the data field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields<'a> {
    pub tag: Tag,
    /// Fields between the tag and the data field (x, k, n, SETID or just SETID).
    pub head: Vec<&'a str>,
    pub data: &'a str,
    /// Fields after the data field (VER if locked, then CHECK).
    pub tail: Vec<&'a str>,
}

/// Splits a colon-form string into tag, head fields, data and tail fields. Returns `None`
/// if the tag is unknown or the field count does not fit the tag.
pub fn split_fields(canon: &str) -> Option<Fields<'_>> {
    let parts: Vec<&str> = canon.split(':').collect();
    let tag = Tag::from_str_exact(parts.first()?)?;
    let (h, tl) = (tag.head(), tag.tail());
    if parts.len() != h + 1 + tl {
        return None;
    }
    Some(Fields {
        tag,
        head: parts[1..h].to_vec(),
        data: parts[h],
        tail: parts[h + 1..].to_vec(),
    })
}

// ------------------------------------------------------------------ parsing

/// Fixes typing slips in the data field. ASCII is replaced by ASCII, so the result has the
/// input's length and its buffer, sized for that, never reallocates.
fn fix_b32(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    out.extend(s.chars().map(|c| match c {
        '0' => 'O',
        '1' => 'I',
        '8' => 'B',
        c => c,
    }));
    out
}

fn fix_hex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    out.extend(s.chars().map(|c| match c {
        'O' => '0',
        'I' | 'L' => '1',
        c => c,
    }));
    out
}

struct Common {
    tag: Tag,
    // Holds the data field in base32, which is a plain share or key for BCP1 and BCPK1.
    parts: Zeroizing<Vec<String>>,
    data: Zeroizing<[u8; DATA_LEN]>,
    ver: Option<String>,
}

fn parse_common(text: &str, kind: StringKind) -> Result<Common, ParseError> {
    let canon = Zeroizing::new(canonical(text));
    let mut parts: Zeroizing<Vec<String>> =
        Zeroizing::new(canon.split(':').map(str::to_string).collect());
    let tag = match Tag::from_str_exact(&parts[0]) {
        Some(t) if t.is_share() == (kind == StringKind::Share) => t,
        _ => return Err(ParseError::NotRecognised { kind }),
    };
    let (h, tl) = (tag.head(), tag.tail());
    if parts.len() != h + 1 + tl {
        return Err(ParseError::WrongFieldCount);
    }
    let fixed = fix_b32(&parts[h]);
    parts[h].zeroize();
    parts[h] = fixed;
    for i in std::iter::once(h - 1).chain(h + 1..h + 1 + tl) {
        parts[i] = fix_hex(&parts[i]);
    }
    let body = Zeroizing::new(parts[..parts.len() - 1].join(":"));
    if check(&body) != parts[parts.len() - 1] {
        return Err(ParseError::ChecksumMismatch);
    }
    let raw = unb32(&parts[h]).ok_or(ParseError::MalformedData)?;
    if raw.len() != DATA_LEN {
        return Err(ParseError::WrongLength);
    }
    let mut data = Zeroizing::new([0u8; DATA_LEN]);
    data.copy_from_slice(&raw);
    let ver = (tl == 2).then(|| parts[h + 1].clone());
    Ok(Common {
        tag,
        parts,
        data,
        ver,
    })
}

/// Plain ASCII decimal without leading zeros ("0" alone is fine). Saturates on huge values
/// so they land in the range check rather than overflowing.
fn parse_decimal(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    Some(
        s.bytes()
            .fold(0u32, |acc, b| {
                acc.saturating_mul(10).saturating_add((b - b'0') as u32)
            })
            .min(1_000_000),
    )
}

/// Parses a share string in any accepted form (colon, space, dashes, lowercase, typo slips).
pub fn parse_share(text: &str) -> Result<ParsedShare, ParseError> {
    let c = parse_common(text, StringKind::Share)?;
    let nums = (
        parse_decimal(&c.parts[1]),
        parse_decimal(&c.parts[2]),
        parse_decimal(&c.parts[3]),
    );
    let (Some(x), Some(k), Some(n)) = nums else {
        return Err(ParseError::MalformedShareFields);
    };
    if !(2 <= k && k <= n && n <= 255 && 1 <= x && x <= n) {
        return Err(ParseError::OutOfRange);
    }
    Ok(ParsedShare {
        tag: c.tag,
        x: x as u8,
        k: k as u8,
        n: n as u8,
        set_id: c.parts[4].clone(),
        data: c.data,
        ver: c.ver,
    })
}

/// Parses a master plate string. Unlocked plates are checked against their set ID.
pub fn parse_master(text: &str) -> Result<ParsedMaster, ParseError> {
    let c = parse_common(text, StringKind::Master)?;
    if c.tag == Tag::Bcpk1 && set_id(&*c.data) != c.parts[1] {
        return Err(ParseError::SetIdMismatch);
    }
    Ok(ParsedMaster {
        tag: c.tag,
        set_id: c.parts[1].clone(),
        data: c.data,
        ver: c.ver,
    })
}

/// True if the canonical form of `text` starts with a master tag.
pub fn is_master(text: &str) -> bool {
    let c = Zeroizing::new(canonical(text));
    c.starts_with("BCPK1:") || c.starts_with("BCPK2:")
}

// ------------------------------------------------------------------ encoding

/// Bytes reserved for an encoded string: the longest tag, three 3-digit numbers, the base32
/// data, the CHECK and the separators (78 in all), plus the set ID and the verifier.
fn encoded_capacity(sid: &str, ver: Option<&str>) -> usize {
    80 + sid.len() + ver.map_or(0, str::len)
}

/// Appends the base32 data, the verifier if any, then `:` and the CHECK of everything before.
/// `out` was sized by [`encoded_capacity`], so it never reallocates and the plain base32 of an
/// unlocked share or key is never left behind in a freed buffer.
fn finish_encoding(mut out: String, data: &[u8; DATA_LEN], ver: Option<&str>) -> String {
    base32().encode_append(data, &mut out);
    if let Some(v) = ver {
        out.push(':');
        out.push_str(v);
    }
    let c = check(&out);
    out.push(':');
    out.push_str(&c);
    out
}

/// Encodes a share in colon form. BCP2 if `ver` is given, else BCP1.
pub fn encode_share(
    x: u8,
    k: u8,
    n: u8,
    sid: &str,
    data: &[u8; DATA_LEN],
    ver: Option<&str>,
) -> String {
    use fmt::Write as _;
    let tag = if ver.is_some() { Tag::Bcp2 } else { Tag::Bcp1 };
    let mut out = String::with_capacity(encoded_capacity(sid, ver));
    // Writing to a `String` cannot fail.
    let _ = write!(out, "{}:{x}:{k}:{n}:{sid}:", tag.as_str());
    finish_encoding(out, data, ver)
}

/// Encodes a master plate in colon form. BCPK2 if `ver` is given, else BCPK1.
pub fn encode_master(sid: &str, data: &[u8; DATA_LEN], ver: Option<&str>) -> String {
    let tag = if ver.is_some() {
        Tag::Bcpk2
    } else {
        Tag::Bcpk1
    };
    let mut out = String::with_capacity(encoded_capacity(sid, ver));
    out.push_str(tag.as_str());
    out.push(':');
    out.push_str(sid);
    out.push(':');
    finish_encoding(out, data, ver)
}

#[cfg(test)]
mod tests {
    //! The no-reallocation rule: a buffer that grows leaves its old contents in freed memory,
    //! so every buffer that can hold plain key material is sized up front. The proof is that
    //! the capacity after the work is still the one reserved.
    use super::*;

    #[test]
    fn upper_case_never_grows_more_than_three_times() {
        for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
            let grown: usize = c.to_uppercase().map(char::len_utf8).sum();
            assert!(grown <= upper_capacity(c.len_utf8()), "{c:?}");
        }
    }

    /// The definitions `clean` and `canonical` had before they were rewritten to avoid
    /// temporaries.
    fn reference_canonical(text: &str) -> String {
        let plain_clean = |t: &str| -> String {
            t.to_uppercase()
                .chars()
                .filter(|&c| !is_py_space(c) && c != '-')
                .collect()
        };
        if text.contains(':') {
            return plain_clean(text);
        }
        let up = text.to_uppercase().replace('-', " ");
        let tokens: Vec<&str> = up.split(is_py_space).filter(|w| !w.is_empty()).collect();
        if let Some(tag) = tokens.first().and_then(|t| Tag::from_str_exact(t)) {
            let (h, tl) = (tag.head(), tag.tail());
            if tokens.len() > h + tl {
                let mut out: Vec<String> = tokens[..h].iter().map(|s| s.to_string()).collect();
                out.push(tokens[h..tokens.len() - tl].concat());
                out.extend(tokens[tokens.len() - tl..].iter().map(|s| s.to_string()));
                return out.join(":");
            }
        }
        plain_clean(text)
    }

    #[test]
    fn canonical_matches_the_plain_definition() {
        for text in [
            "bcp1 1 2 3 abcd1234 aaaa-bbbb cccc 1234",
            "bcp1:1:2:3:ABCD:dd-ee ff:12",
            "bcpk2-abcd1234-qqqq-qqqq-123-abcd",
            "BCPK2 ABCD1234 QQQQ\u{3000}QQQQ\u{1f}123 ABCD",
            "\u{1f}x\u{3000}y-\u{df}\u{149}\u{390}",
            "bcp2 1 2",
            "",
        ] {
            assert_eq!(canonical(text), reference_canonical(text), "{text:?}");
        }
    }

    #[test]
    fn canonical_fills_its_buffers_without_reallocating() {
        // Characters whose upper case is three times longer: the worst case.
        let wide = "\u{390}".repeat(40);
        let c = clean(&wide);
        assert_eq!(c.len(), upper_capacity(wide.len()));
        assert_eq!(c.capacity(), upper_capacity(wide.len()));
        // The space form is joined into a second buffer the size of the upper-cased text.
        let spaced = format!("bcp1 1 2 3 abcd1234 {}1234", "aaaa ".repeat(13));
        let out = canonical(&spaced);
        assert!(out.starts_with("BCP1:1:2:3:ABCD1234:AAAA"));
        assert_eq!(out.capacity(), spaced.len());
    }

    #[test]
    fn group_reserves_enough_up_front() {
        let typed = "A".repeat(52);
        let g = group(&typed, 4);
        assert_eq!(g.len(), 52 + 12);
        assert_eq!(g.capacity(), 52 + 13);
        assert_eq!(group("ABCDEFGHI", 4), "ABCD EFGH I");
        assert_eq!(group("ABC", 0), "A B C");
        assert_eq!(group("\u{e9}\u{e9}\u{e9}", 2), "\u{e9}\u{e9} \u{e9}");
        assert_eq!(group("", 4), "");
    }

    #[test]
    fn encoded_strings_fit_the_reserved_capacity() {
        let sid = "ABCDEF12";
        for ver in [None, Some("ABC")] {
            let s = encode_share(255, 255, 255, sid, &[0xFF; DATA_LEN], ver);
            assert_eq!(s.capacity(), encoded_capacity(sid, ver), "{s}");
            let m = encode_master(sid, &[0xFF; DATA_LEN], ver);
            assert_eq!(m.capacity(), encoded_capacity(sid, ver), "{m}");
        }
        // The text is the same as before it was built in place.
        let s = encode_share(1, 2, 3, sid, &[0; DATA_LEN], None);
        let body = format!("BCP1:1:2:3:{sid}:{}", b32(&[0; DATA_LEN]));
        assert_eq!(s, format!("{body}:{}", check(&body)));
        let m = encode_master(sid, &[0; DATA_LEN], Some("ABC"));
        let body = format!("BCPK2:{sid}:{}:ABC", b32(&[0; DATA_LEN]));
        assert_eq!(m, format!("{body}:{}", check(&body)));
    }

    #[test]
    fn typing_slip_fixes_keep_the_length() {
        let s = fix_b32("A0B1C8D\u{e9}");
        assert_eq!(s, "AOBICBD\u{e9}");
        assert_eq!(s.capacity(), "A0B1C8D\u{e9}".len());
        assert_eq!(fix_hex("OIL9"), "0119");
    }
}
