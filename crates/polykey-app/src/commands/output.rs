//! Shared console output.

use polykey_core::codec::DATA_LEN;
use polykey_core::recover::passphrase;

use super::io::Io;

/// Reference `show_passphrase`. This is the one place the passphrase is printed, by design.
///
/// The lines are written straight to the output stream: building them with `format!` first
/// would leave plain `String` copies of the passphrase that are freed without being wiped.
/// Errors from the stream are ignored, like `Io::line`.
pub fn show_passphrase(io: &mut Io, secret: &[u8; DATA_LEN], heading: &str) {
    let (typed, grouped) = passphrase(secret);
    io.line(&format!("\n{heading}\n"));
    let _ = writeln!(io.out, "   Type exactly (no spaces):  {}", typed.as_str());
    let _ = writeln!(io.out, "   Reading aid:               {}", grouped.as_str());
}
