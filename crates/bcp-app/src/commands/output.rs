//! Shared console output.

use bcp_core::codec::DATA_LEN;
use bcp_core::recover::passphrase;

use super::io::Io;

/// Reference `show_passphrase`. This is the one place the passphrase is printed, by design.
pub fn show_passphrase(io: &mut Io, secret: &[u8; DATA_LEN], heading: &str) {
    let (typed, grouped) = passphrase(secret);
    io.line(&format!("\n{heading}\n"));
    io.line(&format!("   Type exactly (no spaces):  {}", *typed));
    io.line(&format!("   Reading aid:               {}", *grouped));
}
