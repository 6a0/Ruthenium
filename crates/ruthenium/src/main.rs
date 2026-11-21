//! Ruthenium: composition root.

#![forbid(unsafe_code)]

/// Shown on every first-run, About and title screen (Minecraft Usage Guidelines).
pub const DISCLAIMER: &str =
    "NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.";

fn main() {
    println!("Ruthenium {}", env!("CARGO_PKG_VERSION"));
    println!("{DISCLAIMER}");
}
