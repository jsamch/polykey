//! Plate text content: the readable lines engraved next to or under the QR code.
//!
//! Ports `PASS_NOTE`, `_groups`, `share_lines`, `master_lines` and `large_lines` from the
//! reference. All inputs are colon-form plate strings (as produced by the encoder).

use crate::RenderError;
use polykey_core::codec::{group, split_fields, Fields};

/// Note engraved on plates whose data is passcode-locked.
pub const PASS_NOTE: &str = "PASSCODE REQUIRED";

fn fields(text: &str, want_share: bool) -> Result<Fields<'_>, RenderError> {
    match split_fields(text) {
        Some(f) if f.tag.is_share() == want_share => Ok(f),
        _ => Err(RenderError::Invalid(if want_share {
            "plate text is not a well-formed share string".into()
        } else {
            "plate text is not a well-formed master string".into()
        })),
    }
}

pub(crate) fn share_fields(text: &str) -> Result<Fields<'_>, RenderError> {
    fields(text, true)
}

pub(crate) fn master_fields(text: &str) -> Result<Fields<'_>, RenderError> {
    fields(text, false)
}

/// `_groups`: the data in groups of 4 characters, `per` groups per line.
pub fn groups(data: &str, per: usize) -> Vec<String> {
    let per = per.max(1);
    let g: Vec<String> = group(data, 4).split(' ').map(str::to_string).collect();
    g.chunks(per).map(|c| c.join(" ")).collect()
}

/// Title text: the label, plus ` DEMO` on demo sets.
pub(crate) fn title(label: &str, demo: bool) -> String {
    format!("{label}{}", if demo { " DEMO" } else { "" })
}

/// Back of a two-sided share plate: title, info, then data 4 groups per line, then the tail.
pub fn share_lines(share_text: &str, label: &str, demo: bool) -> Result<Vec<String>, RenderError> {
    let f = share_fields(share_text)?;
    let (x, k, n, sid) = share_head(&f)?;
    let mut out = vec![title(label, demo), format!("SHARE {x}/{n}  NEED {k}")];
    if f.tag.is_locked() {
        out.push(PASS_NOTE.to_string());
    }
    out.push(format!("{}:{x}:{k}:{n}:{sid}:", f.tag.as_str()));
    out.extend(groups(f.data, 4));
    out.push(format!(":{}", f.tail.join(":")));
    Ok(out)
}

/// Back of a master plate.
pub fn master_lines(
    master_text: &str,
    label: &str,
    demo: bool,
) -> Result<Vec<String>, RenderError> {
    let f = master_fields(master_text)?;
    let mut out = vec![title(label, demo), "MASTER KEY".to_string()];
    if f.tag.is_locked() {
        out.push(PASS_NOTE.to_string());
    }
    out.push(format!("SET {}", f.head[0]));
    out.extend(groups(f.data, 4));
    out.push(format!(":{}", f.tail.join(":")));
    Ok(out)
}

/// Text under the QR on the 90 mm plate: data 7 groups per line.
pub fn large_lines(share_text: &str) -> Result<Vec<String>, RenderError> {
    let f = share_fields(share_text)?;
    let mut out = Vec::new();
    if f.tag.is_locked() {
        out.push(PASS_NOTE.to_string());
    }
    out.push(format!("{}:{}:", f.tag.as_str(), f.head.join(":")));
    out.extend(groups(f.data, 7));
    out.push(format!(":{}", f.tail.join(":")));
    Ok(out)
}

pub(crate) fn share_head<'a>(
    f: &Fields<'a>,
) -> Result<(&'a str, &'a str, &'a str, &'a str), RenderError> {
    match f.head.as_slice() {
        [x, k, n, sid] => Ok((x, k, n, sid)),
        _ => Err(RenderError::Invalid(
            "plate text is not a well-formed share string".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCKED: &str =
        "BCP2:1:2:3:B2666B51:ZF2KPQTLGZLGNWXZ2BXHY5LTVVM47DVFNJW4JM6Q2MT5B5CL7C5A:862:ABCD";
    const MASTER: &str = "BCPK1:B2666B51:ZF2KPQTLGZLGNWXZ2BXHY5LTVVM47DVFNJW4JM6Q2MT5B5CL7C5A:ABCD";

    #[test]
    fn share_back_lines() {
        let l = share_lines(LOCKED, "BCP KEY", true).unwrap();
        assert_eq!(l[0], "BCP KEY DEMO");
        assert_eq!(l[1], "SHARE 1/3  NEED 2");
        assert_eq!(l[2], PASS_NOTE);
        assert_eq!(l[3], "BCP2:1:2:3:B2666B51:");
        assert_eq!(l[4], "ZF2K PQTL GZLG NWXZ");
        assert_eq!(l.last().unwrap(), ":862:ABCD");
        assert_eq!(l.len(), 9);
    }

    #[test]
    fn large_and_master_lines() {
        let l = large_lines(LOCKED).unwrap();
        assert_eq!(l[0], PASS_NOTE);
        assert_eq!(l[1], "BCP2:1:2:3:B2666B51:");
        assert_eq!(l[2], "ZF2K PQTL GZLG NWXZ 2BXH Y5LT VVM4");
        let m = master_lines(MASTER, "X", false).unwrap();
        assert_eq!(m[..3], ["X", "MASTER KEY", "SET B2666B51"]);
        assert!(share_lines(MASTER, "X", false).is_err());
        assert!(master_lines(LOCKED, "X", false).is_err());
        assert!(large_lines("garbage").is_err());
    }
}
