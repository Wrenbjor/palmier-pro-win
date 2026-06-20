//! # palmier-history
//!
//! Undo/redo stacks with the user stack and the agent stack kept separate
//! (FOUNDATION §4, §6.14 "Agent undo stack"). Operates over `palmier-model` state.
//!
//! Skeleton stub: real stacks land per the history epic.

/// Placeholder for the undo/redo subsystem.
pub fn placeholder() -> &'static str {
    "palmier-history"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-history");
    }
}
