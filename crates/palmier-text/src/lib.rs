//! # palmier-text
//!
//! Text layout/shaping, font registry, and caption styling (FOUNDATION §4, §6.6, §6.9).
//! Wraps `cosmic-text` (layout + shaping) and `fontdb` (system + bundled font
//! discovery); the font-registration hook is also called from the boot path (E1-S5).
//! Heavy deps are added per-story, not in this skeleton.

/// Register bundled + system fonts. Skeleton no-op; real impl lands per Epic 5 / E1-S5.
pub fn register_bundled_fonts() {
    // no-op placeholder
}

#[cfg(test)]
mod tests {
    #[test]
    fn register_is_callable() {
        super::register_bundled_fonts();
    }
}
