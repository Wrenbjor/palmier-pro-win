//! # palmier-edit
//!
//! Pure editing engines: ripple/overwrite, snap, trim, split (FOUNDATION §4, §6.4).
//! These are pure functions over the `palmier-model` shapes with no UI dependency.
//!
//! Skeleton stub: real `RippleEngine` / `OverwriteEngine` / `SnapEngine` land per Epic 3.

/// Placeholder for the editing engines.
pub fn placeholder() -> &'static str {
    "palmier-edit"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-edit");
    }
}
