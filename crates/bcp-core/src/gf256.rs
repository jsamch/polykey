//! GF(256) arithmetic, AES field (polynomial 0x11B, generator 3), with the same EXP and LOG
//! tables as the reference. Tables are built at compile time.

const fn build() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        // Multiply by 3.
        x ^= (x << 1) ^ if x & 0x80 != 0 { 0x11B } else { 0 };
        x &= 0xFF;
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    (exp, log)
}

const TABLES: ([u8; 512], [u8; 256]) = build();
static EXP: [u8; 512] = TABLES.0;
static LOG: [u8; 256] = TABLES.1;

/// The 512-entry EXP table (`LOG[0]` of the companion table is 0 and unused).
pub fn exp_table() -> &'static [u8; 512] {
    &EXP
}

/// The 256-entry LOG table.
pub fn log_table() -> &'static [u8; 256] {
    &LOG
}

/// Field multiplication.
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    EXP[LOG[a as usize] as usize + LOG[b as usize] as usize]
}

/// Field division. Returns `None` when `b` is zero (the reference raises `ZeroDivisionError`).
pub fn div(a: u8, b: u8) -> Option<u8> {
    if b == 0 {
        return None;
    }
    if a == 0 {
        return Some(0);
    }
    Some(EXP[(LOG[a as usize] as usize + 255 - LOG[b as usize] as usize) % 255])
}
