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
        // The reference (Python base64) ignores non-zero trailing bits; so do we.
        spec.check_trailing_bits = false;
        spec.encoding()
            .expect("static base32 specification is valid")
    })
}

/// RFC 4648 base32, uppercase, padding stripped.
pub fn b32(data: &[u8]) -> String {
    base32().encode(data)
}

/// Decodes unpadded base32. Explicit `=` padding is rejected (strict rule), as is any
/// character outside `A-Z2-7`. The input is expected to be uppercase already.
pub fn unb32(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    base32().decode(text.as_bytes()).ok().map(Zeroizing::new)
}

/// BCP1 set ID: first 8 uppercase hex characters of SHA-256(secret).
pub fn set_id(secret: &[u8]) -> String {
    hex_upper(&Sha256::digest(secret), 8)
}

/// Verifier: first 3 uppercase hex characters of SHA-256("BCP2-verifier|" + secret).
pub fn verifier(secret: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"BCP2-verifier|");
    h.update(secret);
    hex_upper(&h.finalize(), 3)
}

/// CHECK: first 4 uppercase hex characters of SHA-256 of the body's UTF-8 bytes.
pub fn check(body: &str) -> String {
    hex_upper(&Sha256::digest(body.as_bytes()), 4)
}

/// Splits `s` into groups of `size` characters joined by single spaces (`size` 0 acts as 1).
pub fn group(s: &str, size: usize) -> String {
    let size = size.max(1);
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(size)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

// ------------------------------------------------------------------ canonical form

/// Whitespace as Python's `str.split()` sees it (Unicode White_Space plus U+001C..U+001F).
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn clean(text: &str) -> String {
    text.to_uppercase()
        .chars()
        .filter(|&c| !is_py_space(c) && c != '-')
        .collect()
}

/// Normalises any accepted form to the colon form used for checksums. Only upper-cases and
/// removes spaces and dashes (or joins space-form tokens); typing slips are fixed in parsing.
pub fn canonical(text: &str) -> String {
    if text.contains(':') {
        return clean(text);
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

fn fix_b32(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '0' => 'O',
            '1' => 'I',
            '8' => 'B',
            c => c,
        })
        .collect()
}

fn fix_hex(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
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
    let c = canonical(text);
    c.starts_with("BCPK1:") || c.starts_with("BCPK2:")
}

// ------------------------------------------------------------------ encoding

/// Encodes a share in colon form. BCP2 if `ver` is given, else BCP1.
pub fn encode_share(
    x: u8,
    k: u8,
    n: u8,
    sid: &str,
    data: &[u8; DATA_LEN],
    ver: Option<&str>,
) -> String {
    let tag = if ver.is_some() { Tag::Bcp2 } else { Tag::Bcp1 };
    let mut body = format!("{}:{x}:{k}:{n}:{sid}:{}", tag.as_str(), b32(data));
    if let Some(v) = ver {
        body.push(':');
        body.push_str(v);
    }
    let c = check(&body);
    format!("{body}:{c}")
}

/// Encodes a master plate in colon form. BCPK2 if `ver` is given, else BCPK1.
pub fn encode_master(sid: &str, data: &[u8; DATA_LEN], ver: Option<&str>) -> String {
    let tag = if ver.is_some() {
        Tag::Bcpk2
    } else {
        Tag::Bcpk1
    };
    let mut body = format!("{}:{sid}:{}", tag.as_str(), b32(data));
    if let Some(v) = ver {
        body.push(':');
        body.push_str(v);
    }
    let c = check(&body);
    format!("{body}:{c}")
}
