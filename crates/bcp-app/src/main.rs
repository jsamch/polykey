//! The `bcp` binary: command line interface, with the GUI behind the `gui` feature.

fn main() {
    // No arguments, `--version` and `-V` all print the version for now.
    println!("bcp {}", env!("CARGO_PKG_VERSION"));
}
