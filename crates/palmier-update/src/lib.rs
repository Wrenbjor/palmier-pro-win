//! # palmier-update
//!
//! Tauri 2 updater glue — Ed25519-signed manifest check, `update_available` /
//! `update_version` surfacing, single `stable` channel (FOUNDATION §4, §8.4; E1-S10).
//! Silently disables when no signed feed is present. Updater deps are added
//! per-story, not in this skeleton.

/// v1 update channel (OQ-1 working decision).
pub const CHANNEL: &str = "stable";

#[cfg(test)]
mod tests {
    #[test]
    fn channel_is_stable() {
        assert_eq!(super::CHANNEL, "stable");
    }
}
