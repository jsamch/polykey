//! `bcp selftest` (reference `cmd_selftest`): built-in checks that involve no real secrets.
//! Each check catches its own failure, including panics, so one bad check reports FAIL and the
//! rest still run, like the reference try/except.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use bcp_core::codec::{
    encode_master, encode_share, group, parse_master, parse_share, qr_payload, set_id, verifier,
    DATA_LEN,
};
use bcp_core::gf256::{div, mul};
use bcp_core::lock::{kdf_stream, lock, KdfCost, Passcode, Role};
use bcp_core::shamir::{combine, split, CoeffRng, OsRng, Share, SECRET_LEN};
use zeroize::Zeroizing;

use super::io::Io;
use crate::error::CliError;

/// A check returns an optional note on success or a failure message.
pub type CheckFn = fn(KdfCost) -> Result<String, String>;

type Secret = Zeroizing<[u8; SECRET_LEN]>;

const SKIP_QR: &str = "skipped: QR support not built yet (Phases 4 and 5)";

/// The checks in reference order.
pub fn checks() -> Vec<(&'static str, CheckFn)> {
    vec![
        ("GF(256) arithmetic", field),
        ("Shamir split/combine (2-of-2 to 5-of-8)", shamir),
        ("share and master encoding", encoding),
        ("tampered share rejected", tamper),
        ("0/1/8 typing slips tolerated", typos),
        ("passcode lock and unlock", passcode_lock),
        (
            "scrypt available at full strength (needs about 256 MB)",
            kdf_real,
        ),
        ("QR generate and decode", qr_roundtrip),
    ]
}

pub fn run_selftest(io: &mut Io, cost: KdfCost) -> Result<u8, CliError> {
    Ok(run_checks(io, &checks(), cost))
}

/// Runs the given checks and prints the report. Returns the exit code.
pub fn run_checks(io: &mut Io, list: &[(&'static str, CheckFn)], cost: KdfCost) -> u8 {
    let mut results = Vec::new();
    for (name, f) in list {
        let res = match catch_unwind(AssertUnwindSafe(|| f(cost))) {
            Ok(Ok(note)) => ("PASS", note),
            Ok(Err(msg)) => ("FAIL", msg),
            Err(p) => ("FAIL", panic_message(p.as_ref())),
        };
        results.push((*name, res.0, res.1));
    }
    io.line(&format!(
        "bcp {} (Rust {}), QR backend: none, scan test: no",
        env!("CARGO_PKG_VERSION"),
        env!("BCP_RUSTC_VERSION")
    ));
    io.line(&format!(
        "bcp-core {} (format version {})",
        env!("CARGO_PKG_VERSION"),
        bcp_core::FORMAT_VERSION
    ));
    let mut fails = 0usize;
    for (name, res, msg) in &results {
        let skipped = *res == "FAIL" && msg.starts_with("skipped");
        if !(*res == "PASS" || skipped) {
            fails += 1;
        }
        let label = if skipped { "SKIP" } else { res };
        let note = if msg.is_empty() {
            String::new()
        } else {
            format!("  ({msg})")
        };
        io.line(&format!("  {label}  {name}{note}"));
    }
    if fails == 0 {
        io.line("\nAll tests passed.");
    } else {
        io.line(&format!("\n{fails} test(s) FAILED. Do not use this setup."));
    }
    u8::from(fails != 0)
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "check panicked".to_owned()
    }
}

macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !($cond) {
            return Err(format!($($msg)+));
        }
    };
}

fn random_secret() -> Secret {
    let mut s = Zeroizing::new([0u8; SECRET_LEN]);
    OsRng.fill(s.as_mut());
    s
}

/// All k-element index subsets of 0..n in lexicographic order.
fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut cur = Vec::with_capacity(k);
    fn rec(start: usize, n: usize, k: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if cur.len() == k {
            out.push(cur.clone());
            return;
        }
        for i in start..n {
            cur.push(i);
            rec(i + 1, n, k, cur, out);
            cur.pop();
        }
    }
    rec(0, n, k, &mut cur, &mut out);
    out
}

fn pick(shares: &[Share], idx: &[usize]) -> Vec<Share> {
    idx.iter().map(|&i| shares[i].clone()).collect()
}

fn split_ok(sec: &[u8; SECRET_LEN], k: u8, n: u8) -> Result<Vec<Share>, String> {
    split(sec, k, n, &mut OsRng).map_err(|e| e.to_string())
}

fn field(_: KdfCost) -> Result<String, String> {
    for v in 1..=255u8 {
        let inv = div(1, v).ok_or_else(|| format!("inverse failed for {v}"))?;
        ensure!(mul(v, inv) == 1, "inverse failed for {v}");
    }
    ensure!(mul(0x57, 0x83) == 0xC1, "FIPS-197 multiply vector");
    Ok(String::new())
}

fn shamir(_: KdfCost) -> Result<String, String> {
    for (k, n) in [(2u8, 2u8), (2, 3), (3, 5), (5, 8)] {
        let sec = random_secret();
        let sh = split_ok(&sec, k, n)?;
        for c in combinations(usize::from(n), usize::from(k)) {
            let got = combine(&pick(&sh, &c)).map_err(|e| e.to_string())?;
            ensure!(*got == *sec, "{k}-of-{n} failed");
        }
        if k > 2 {
            for c in combinations(usize::from(n), usize::from(k) - 1) {
                let got = combine(&pick(&sh, &c)).map_err(|e| e.to_string())?;
                ensure!(*got != *sec, "k-1 shares must not rebuild the key");
            }
        }
    }
    Ok(String::new())
}

fn encoding(_: KdfCost) -> Result<String, String> {
    let sec = random_secret();
    let sid = set_id(&*sec);
    let sh = split_ok(&sec, 3, 5)?;
    let (x, data) = (sh[2].x, sh[2].y.clone());
    let want_ver = verifier(&*sec);
    for ver in [None, Some(want_ver.as_str())] {
        let s = encode_share(x, 3, 5, &sid, &data, ver);
        let same = |text: &str| -> Result<bool, String> {
            let p = parse_share(text).map_err(|e| e.to_string())?;
            Ok((p.x, p.k, p.n) == (x, 3, 5)
                && p.set_id == sid
                && *p.data == *data
                && p.ver.as_deref() == ver)
        };
        ensure!(same(&s)?, "share round trip");
        let spaced = format!(" {} ", group(&s.to_lowercase(), 5));
        let p = parse_share(&spaced).map_err(|e| e.to_string())?;
        ensure!(*p.data == *data, "spacing/case");
        ensure!(same(&qr_payload(&s))?, "space-form share");
        ensure!(
            !qr_payload(&s).contains(':'),
            "QR payload must not look like a link"
        );
        let m = encode_master(&sid, &sec, ver);
        let master_same = |text: &str| -> Result<bool, String> {
            let p = parse_master(text).map_err(|e| e.to_string())?;
            Ok(p.set_id == sid && *p.data == *sec && p.ver.as_deref() == ver)
        };
        ensure!(master_same(&m)?, "master round trip");
        ensure!(master_same(&qr_payload(&m))?, "space-form master");
    }
    Ok(String::new())
}

fn tamper(_: KdfCost) -> Result<String, String> {
    let sec = random_secret();
    let sh = split_ok(&sec, 2, 3)?;
    let s = encode_share(sh[0].x, 2, 3, &set_id(&*sec), &sh[0].y, None);
    let (body, chk) = s.rsplit_once(':').ok_or("malformed test string")?;
    let i = body.len() - 3;
    let bytes = body.as_bytes();
    let repl = if bytes[i] != b'A' { "A" } else { "B" };
    let flipped = format!("{}{}{}:{}", &body[..i], repl, &body[i + 1..], chk);
    ensure!(parse_share(&flipped).is_err(), "altered share was accepted");
    Ok(String::new())
}

fn typos(_: KdfCost) -> Result<String, String> {
    for _ in 0..200 {
        let sec = random_secret();
        let sh = split_ok(&sec, 2, 3)?;
        let s = encode_share(sh[0].x, 2, 3, &set_id(&*sec), &sh[0].y, None);
        let mut p: Vec<String> = s.split(':').map(str::to_owned).collect();
        if p[5].contains(['O', 'I', 'B']) {
            p[5] = p[5].replace('O', "0").replace('I', "1").replace('B', "8");
            let got = parse_share(&p.join(":")).map_err(|e| e.to_string())?;
            ensure!(*got.data == *sh[0].y, "typing slips changed the data");
            return Ok(String::new());
        }
    }
    Err("no sample with O/I/B found".to_owned())
}

fn kdf_err(e: impl ToString) -> String {
    e.to_string()
}

fn passcode_lock(_: KdfCost) -> Result<String, String> {
    // Fast KDF setting for the logic test; the real setting is timed separately.
    let fast = KdfCost::from_log_n(10);
    let good = Passcode::from("correct horse");
    let bad = Passcode::from("correct horsf");
    let sec = random_secret();
    let mut sid_raw = [0u8; 4];
    OsRng.fill(&mut sid_raw);
    let sid: String = sid_raw.iter().map(|b| format!("{b:02X}")).collect();
    let ver = verifier(&*sec);
    let sh = split_ok(&sec, 3, 5)?;
    let mut locked: Vec<(u8, Zeroizing<[u8; DATA_LEN]>)> = Vec::new();
    for s in &sh {
        let l = lock(&s.y, &good, &sid, Role::Share(s.x), fast).map_err(kdf_err)?;
        ensure!(*l != *s.y, "lock must change the data");
        locked.push((s.x, l));
    }
    let open = |p: &Passcode| -> Result<Vec<Share>, String> {
        let mut v = Vec::new();
        for x in [1u8, 3, 5] {
            let (_, l) = locked
                .iter()
                .find(|(lx, _)| *lx == x)
                .ok_or("missing share")?;
            let d = lock(l, p, &sid, Role::Share(x), fast).map_err(kdf_err)?;
            v.push(Share::new(x, *d));
        }
        Ok(v)
    };
    let rebuilt = combine(&open(&good)?).map_err(|e| e.to_string())?;
    ensure!(
        *rebuilt == *sec && verifier(&*rebuilt) == ver,
        "unlock failed"
    );
    let wrong = combine(&open(&bad)?).map_err(|e| e.to_string())?;
    ensure!(*wrong != *sec, "wrong passcode must not rebuild the key");
    let other = Passcode::from("other pass");
    let m = lock(&sec, &other, &sid, Role::Master, fast).map_err(kdf_err)?;
    let back = lock(&m, &other, &sid, Role::Master, fast).map_err(kdf_err)?;
    ensure!(*back == *sec, "master unlock failed");
    let cross = lock(&m, &good, &sid, Role::Master, fast).map_err(kdf_err)?;
    ensure!(*cross != *sec, "passcodes are separate");
    // The reference uses the role "x" here; Role cannot express it, so Role::Master stands in.
    // The point is that NFC and NFD spellings give the same mask.
    let a = kdf_stream(&Passcode::from("caf\u{e9}"), &sid, Role::Master, fast).map_err(kdf_err)?;
    let b =
        kdf_stream(&Passcode::from("cafe\u{301}"), &sid, Role::Master, fast).map_err(kdf_err)?;
    ensure!(
        *a == *b,
        "accented passcodes must match however the keyboard composes them"
    );
    Ok(String::new())
}

/// `cost` is FULL in the binary; tests may pass a smaller one.
fn kdf_real(cost: KdfCost) -> Result<String, String> {
    let t0 = Instant::now();
    kdf_stream(&Passcode::from("timing"), "00000000", Role::Share(1), cost).map_err(kdf_err)?;
    Ok(format!("{:.2} s per unlock", t0.elapsed().as_secs_f64()))
}

fn qr_roundtrip(_: KdfCost) -> Result<String, String> {
    Err(SKIP_QR.to_owned())
}
