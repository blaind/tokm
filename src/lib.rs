//! Placeholder crate for Tokm.
//!
//! No functionality yet — this release exists to reserve the name on crates.io.

/// Version of this placeholder release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION, "0.0.1");
    }
}
