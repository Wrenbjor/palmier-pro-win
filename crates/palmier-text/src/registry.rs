//! Font discovery — the bundled reference fonts + the user's installed system
//! fonts (FOUNDATION §6.6: "`fontdb` for font discovery (includes user-installed
//! fonts on Windows + Linux)").
//!
//! The reference ships a fixed set of font families in
//! `Sources/PalmierPro/Resources/Fonts/`; we bundle the **same** `.ttf` files
//! (copied into `assets/fonts/`, GPLv3/OFL boundary per OQ-11 / R-2 — the OFL +
//! LICENSE files travel with them) and embed them in the binary via
//! [`include_bytes!`] so a caption renders identically regardless of what the host
//! has installed. System fonts are loaded on top so a user can pick any installed
//! family in the Inspector.

use cosmic_text::{fontdb, FontSystem};

/// One bundled font: its family label (for diagnostics) and the embedded bytes.
pub struct BundledFont {
    /// Human-readable family/style label (matches the reference's folder name).
    pub label: &'static str,
    /// The embedded `.ttf` bytes.
    pub bytes: &'static [u8],
}

macro_rules! bundled {
    ($label:literal, $path:literal) => {
        BundledFont {
            label: $label,
            bytes: include_bytes!(concat!("../assets/fonts/", $path)),
        }
    };
}

/// The bundled reference font set (same files the macOS reference ships). Embedded
/// at compile time so they are always available, even on a stripped host.
pub static BUNDLED_FONTS: &[BundledFont] = &[
    bundled!("Anton", "Anton-Regular.ttf"),
    bundled!("BasementGrotesque-Black", "BasementGrotesque-Black.ttf"),
    bundled!("BebasNeue", "BebasNeue-Regular.ttf"),
    bundled!("Caveat", "Caveat[wght].ttf"),
    bundled!("DMSans", "DMSans[opsz,wght].ttf"),
    bundled!("Geist", "Geist[wght].ttf"),
    bundled!("Geist-Italic", "Geist-Italic[wght].ttf"),
    bundled!("GeistMono", "GeistMono[wght].ttf"),
    bundled!("GeistMono-Italic", "GeistMono-Italic[wght].ttf"),
    bundled!("Inter", "Inter[opsz,wght].ttf"),
    bundled!("PermanentMarker", "PermanentMarker-Regular.ttf"),
    bundled!("PlayfairDisplay", "PlayfairDisplay[wght].ttf"),
    bundled!("Poppins", "Poppins-Regular.ttf"),
    bundled!("Poppins-Bold", "Poppins-Bold.ttf"),
    bundled!("Poppins-Italic", "Poppins-Italic.ttf"),
    bundled!("Poppins-BoldItalic", "Poppins-BoldItalic.ttf"),
    bundled!("Shrikhand", "Shrikhand-Regular.ttf"),
    bundled!("SpaceGrotesk", "SpaceGrotesk[wght].ttf"),
];

/// The font registry: owns a cosmic-text [`FontSystem`] (the single source of font
/// data + the fallback chain + the shaping scratch buffer). Construct once and
/// share by `&mut` for every layout — building it is expensive (it scans the
/// system font directories).
pub struct FontRegistry {
    font_system: FontSystem,
}

impl FontRegistry {
    /// A registry with **only** the bundled reference fonts loaded (no system
    /// scan). Deterministic + fast — used in tests and on a minimal host where the
    /// system font directories are empty or untrusted.
    pub fn bundled_only() -> Self {
        let sources = BUNDLED_FONTS
            .iter()
            .map(|f| fontdb::Source::Binary(std::sync::Arc::new(f.bytes.to_vec())));
        let font_system = FontSystem::new_with_fonts(sources);
        FontRegistry { font_system }
    }

    /// A registry with system fonts **plus** the bundled reference fonts (the
    /// production preview registry). The bundled fonts are loaded after the system
    /// scan so the reference families are always present.
    pub fn with_bundled_fonts() -> Self {
        let mut font_system = FontSystem::new();
        let db = font_system.db_mut();
        for f in BUNDLED_FONTS {
            db.load_font_data(f.bytes.to_vec());
        }
        FontRegistry { font_system }
    }

    /// Mutable access to the underlying [`FontSystem`] (cosmic-text needs `&mut`
    /// for shaping; the GPU text pass also needs it to drive its `SwashCache`).
    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// The number of font faces the registry knows about (bundled + system).
    pub fn face_count(&self) -> usize {
        self.font_system.db().len()
    }

    /// Whether a face whose family name contains `name` (case-insensitive) is
    /// registered. The reference resolves a font by exact PostScript/family name
    /// and falls back to the bold system font; the engine's layout uses the same
    /// fallback, but callers (e.g. the Inspector font picker) can probe first.
    pub fn has_family(&self, name: &str) -> bool {
        let needle = name.to_ascii_lowercase();
        self.font_system.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(fam, _)| fam.to_ascii_lowercase().contains(&needle))
        })
    }
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::with_bundled_fonts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_only_loads_every_bundled_face() {
        let reg = FontRegistry::bundled_only();
        // Each embedded file registers at least one face.
        assert!(
            reg.face_count() >= BUNDLED_FONTS.len(),
            "expected ≥ {} faces, got {}",
            BUNDLED_FONTS.len(),
            reg.face_count()
        );
    }

    #[test]
    fn bundled_families_are_queryable() {
        let reg = FontRegistry::bundled_only();
        // A couple of distinctive reference families must resolve by name.
        assert!(reg.has_family("Anton"), "Anton not registered");
        assert!(reg.has_family("Poppins"), "Poppins not registered");
        assert!(reg.has_family("Bebas"), "Bebas Neue not registered");
    }

    #[test]
    fn with_bundled_includes_bundled_even_if_system_empty() {
        // Even when a CI box has no system fonts, the bundled set is present.
        let reg = FontRegistry::with_bundled_fonts();
        assert!(reg.has_family("Inter"));
    }
}
