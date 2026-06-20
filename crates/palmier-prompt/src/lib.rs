//! # palmier-prompt
//!
//! The **single source of the verbatim agent system prompt** ([`AGENT_INSTRUCTIONS`]).
//!
//! Ruling #2 (and the §13 risk note R-5): the agent prompt is **load-bearing
//! contract text** the LLM was tuned against. It is injected at **two** sites —
//! the MCP server's `initialize` `instructions` field (`palmier-mcp`, for external
//! clients: Claude Desktop / Claude Code / Cursor / Codex) and the in-app
//! Anthropic-backed agent's `system` parameter (`palmier-agent`). The reference
//! uses the **same** string in both paths; if a porter "improves" one copy but not
//! the other, the two consumers silently diverge and SM-8 client-compat breaks.
//!
//! This crate exists so there is **exactly one** copy of that string. Both consumers
//! import `palmier_prompt::AGENT_INSTRUCTIONS`; neither carries its own copy.
//!
//! ## Byte fidelity
//!
//! The text is stored as a plain `.txt` (reviewable as text, no Rust-string
//! escaping) and embedded via [`include_str!`]. The `.txt` is **LF-pinned** in
//! `.gitattributes` (`eol=lf`) so a Windows checkout cannot rewrite it to CRLF and
//! change the byte length. The Swift `\`-line-continuations of the reference
//! `AgentInstructions.serverInstructions` are **already resolved** in this file —
//! it is the final shipped text, not the authoring form. Unicode glyphs
//! (`×` U+00D7, `–` U+2013, `•` U+2022, `…` U+2026) are preserved as UTF-8 and must
//! **not** be ASCII-folded. The `tests` module is the drift tripwire: any change to
//! the bytes (length, opening/closing lines, section headers, or the glyph counts)
//! fails CI.

/// The **verbatim** agent system prompt (ruling #2), ported byte-for-byte from the
/// reference `AgentInstructions.serverInstructions` / the
/// `docs/reference/agent-instructions.md` VERBATIM block.
///
/// This is the value placed into:
/// - the MCP `initialize` result's `instructions` field (`palmier-mcp`), and
/// - the Anthropic request's top-level `system` field (`palmier-agent`).
///
/// The literal product token `palmier-pro` (lowercase, hyphen) stays as written —
/// it is the model-facing identity, unchanged by the Windows rebrand.
pub const AGENT_INSTRUCTIONS: &str = include_str!("agent_instructions.txt");

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact byte length of the ported prompt. The single strongest drift tripwire:
    /// any added/removed character, any CRLF rewrite, any ASCII-fold of a glyph
    /// changes this number and fails CI. (8694 UTF-8 bytes / 8633 chars.)
    #[test]
    fn byte_length_is_pinned() {
        assert_eq!(
            AGENT_INSTRUCTIONS.len(),
            8694,
            "agent prompt byte length drifted — the verbatim contract text changed \
             (or a CRLF/encoding rewrite occurred). Restore byte fidelity (ruling #2)."
        );
        // Char count is a second, encoding-independent witness: it pins how many of
        // those bytes are multi-byte glyphs, so an ASCII-fold (which would keep some
        // byte counts close) still trips here.
        assert_eq!(
            AGENT_INSTRUCTIONS.chars().count(),
            8633,
            "agent prompt char count drifted (possible ASCII-fold of a Unicode glyph)."
        );
    }

    /// No CRLF must ever reach the embedded bytes — the `.gitattributes` `eol=lf`
    /// pin enforces this on checkout; this asserts it at compile-of-test time too.
    #[test]
    fn line_endings_are_lf_only() {
        assert!(
            !AGENT_INSTRUCTIONS.contains('\r'),
            "agent prompt contains a CR byte — the LF pin (.gitattributes eol=lf) was \
             defeated; CRLF would change the byte length and alter the prompt."
        );
    }

    /// The exact opening line (reference first line). The prompt names `palmier-pro`
    /// here — that literal product token must survive verbatim.
    #[test]
    fn opening_line_is_verbatim() {
        assert!(
            AGENT_INSTRUCTIONS.starts_with(
                "You are a creative AI assistant connected to palmier-pro, an AI-native \
                 video editor. Help the user build and edit their project by calling the \
                 tools this server exposes."
            ),
            "agent prompt opening line drifted from the verbatim reference."
        );
    }

    /// The exact closing line + trailing newline (the file ends with `\n`).
    #[test]
    fn closing_line_is_verbatim() {
        assert!(
            AGENT_INSTRUCTIONS.ends_with(
                "- When the user is vague about aesthetic direction, ask one focused \
                 question instead of guessing.\n"
            ),
            "agent prompt closing line / trailing newline drifted."
        );
    }

    /// Every behavioral-contract section header, in order. A missing or reordered
    /// header means the prompt structure changed.
    #[test]
    fn all_section_headers_present_in_order() {
        const HEADERS: [&str; 7] = [
            "# Core model",
            "# Always do",
            "# Editing",
            "# Generation",
            "# Audio generation",
            "# Prompt craft",
            "# Communication",
        ];
        let mut cursor = 0usize;
        for header in HEADERS {
            let pos = AGENT_INSTRUCTIONS[cursor..]
                .find(header)
                .unwrap_or_else(|| panic!("missing section header {header} (or out of order)"));
            cursor += pos + header.len();
        }
    }

    /// The Unicode glyphs survive as UTF-8 with their exact counts — proves no
    /// ASCII-fold (`×`→`x`, `–`→`-`, `•`→`*`, `…`→`...`) crept in.
    #[test]
    fn unicode_glyphs_preserved_with_exact_counts() {
        let count = |c: char| AGENT_INSTRUCTIONS.matches(c).count();
        assert_eq!(count('\u{00D7}'), 1, "× U+00D7 (multiplication sign) count drifted");
        assert_eq!(count('\u{2013}'), 2, "– U+2013 (en dash) count drifted");
        assert_eq!(count('\u{2022}'), 8, "• U+2022 (bullet) count drifted");
        assert_eq!(count('\u{2026}'), 2, "… U+2026 (horizontal ellipsis) count drifted");
    }

    /// The literal product token stays lowercase-hyphen `palmier-pro` (not the
    /// rebranded "Palmier Pro Windows") — model-facing identity is unchanged.
    #[test]
    fn product_token_is_lowercase_hyphen() {
        assert!(AGENT_INSTRUCTIONS.contains("palmier-pro"));
    }
}
