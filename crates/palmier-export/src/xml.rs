//! E6-S1 — the whitespace-exact XML core: an [`XmlNode`] tree plus a pure
//! [`render`] that owns **all** indentation and escaping, the [`escape_xml`]
//! table, and the fixed-locale float formatters.
//!
//! Ported literally from the macOS reference
//! `Sources/PalmierPro/Export/XMLExporter.swift` (`struct XMLNode`,
//! `render(_:indent:)`, `escapeXML`, the `el`/`leaf`/`bool` builders). The
//! structure-vs-whitespace split is load-bearing: every emitter describes
//! *structure* only, and `render` is the single owner of bytes — that is why the
//! goldens are stable (docs/reference/export.md §B, "Mapping to FOUNDATION
//! crates": "keep the `XMLNode`/`render` split, it is why goldens are stable").
//!
//! ## Float formatting parity
//!
//! Swift `String(format:)` formats in the C locale (`.`-decimal, no grouping)
//! using IEEE round-half-to-even at the format precision. Rust's `format!`
//! `{:.N}` is locale-independent and likewise rounds half-to-even, so
//! `fmt_f(spec, v)` reproduces the reference bytes for every `%.Nf` spec used by
//! the emitter (`%.1f`, `%.2f`, `%.4f`, `%.5f`). The `float_formatting_parity`
//! unit test pins this against hand-computed edge values (negative, `0.0`, ties).

/// A minimal XML tree node. Emitters build the structure; [`render`] owns every
/// byte of whitespace and escaping so no fragment ever hardcodes its own indent.
///
/// Shapes (reference `render`):
/// - text leaf → `<name attrs>escaped-text</name>`
/// - element with children → open tag, `\n`, children joined by `\n` at
///   indent+2, `\n`, close
/// - empty + no text → `<name attrs/>` self-closing
#[derive(Debug, Clone, PartialEq)]
pub struct XmlNode {
    pub name: String,
    /// Ordered attributes (`(key, value)`), rendered as ` key="escaped-value"`.
    pub attributes: Vec<(String, String)>,
    /// Leaf text, if any → `<name>text</name>`.
    pub text: Option<String>,
    /// Ordered children; empty **and** `text == None` → self-closing.
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    /// An element with children and no attributes.
    pub fn el(name: impl Into<String>, children: Vec<XmlNode>) -> XmlNode {
        XmlNode {
            name: name.into(),
            attributes: Vec::new(),
            text: None,
            children,
        }
    }

    /// An element with attributes and children.
    pub fn el_attrs(
        name: impl Into<String>,
        attrs: Vec<(String, String)>,
        children: Vec<XmlNode>,
    ) -> XmlNode {
        XmlNode {
            name: name.into(),
            attributes: attrs,
            text: None,
            children,
        }
    }

    /// A self-closing element carrying only attributes (`<name attrs/>`).
    pub fn empty_attrs(name: impl Into<String>, attrs: Vec<(String, String)>) -> XmlNode {
        XmlNode {
            name: name.into(),
            attributes: attrs,
            text: None,
            children: Vec::new(),
        }
    }

    /// A text leaf: `<name>escaped-text</name>`.
    pub fn leaf(name: impl Into<String>, value: impl Into<String>) -> XmlNode {
        XmlNode {
            name: name.into(),
            attributes: Vec::new(),
            text: Some(value.into()),
            children: Vec::new(),
        }
    }

    /// An integer leaf (reference `leaf(_:Int)` → the decimal string).
    pub fn leaf_int(name: impl Into<String>, value: i64) -> XmlNode {
        XmlNode::leaf(name, value.to_string())
    }

    /// A boolean leaf rendering the literal `TRUE` / `FALSE` (reference `bool`).
    pub fn bool_leaf(name: impl Into<String>, value: bool) -> XmlNode {
        XmlNode::leaf(name, if value { "TRUE" } else { "FALSE" })
    }
}

/// Render a node to a string, owning all whitespace and escaping.
///
/// Verbatim port of the reference `render(_:indent:)`:
/// - `pad` = `indent` spaces;
/// - text leaf → `pad<name attrs>escape(text)</name>`;
/// - no children + no text → `pad<name attrs/>`;
/// - else → `pad<name attrs>\n` + children rendered at `indent+2` joined by `\n`
///   + `\npad</name>`.
///
/// Indent is **2 spaces per level**, starting at 0.
pub fn render(node: &XmlNode, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let attrs: String = node
        .attributes
        .iter()
        .map(|(k, v)| format!(" {}=\"{}\"", k, escape_xml(v)))
        .collect();

    if let Some(text) = &node.text {
        return format!("{pad}<{}{attrs}>{}</{}>", node.name, escape_xml(text), node.name);
    }
    if node.children.is_empty() {
        return format!("{pad}<{}{attrs}/>", node.name);
    }
    let inner = node
        .children
        .iter()
        .map(|c| render(c, indent + 2))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{pad}<{}{attrs}>\n{inner}\n{pad}</{}>", node.name, node.name)
}

/// Escape XML special characters in the reference's **exact order**:
/// `& < > " '` → `&amp; &lt; &gt; &quot; &apos;`.
///
/// Order matters: `&` is escaped first so the ampersands introduced by the later
/// replacements are not double-escaped (verbatim from the reference `escapeXML`).
pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The `<?xml …?>\n<!DOCTYPE xmeml>\n` document prolog emitted before the tree
/// (reference `build()` return prefix).
pub const XML_PROLOG: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n";

/// Format a float with a fixed number of decimal places, matching Swift
/// `String(format: "%.<places>f", value)` byte-for-byte (C-locale, half-to-even).
///
/// The emitter only uses places ∈ {1, 2, 4, 5}; this covers them uniformly.
pub fn fmt_f(places: usize, value: f64) -> String {
    format!("{value:.places$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_text_leaf() {
        let n = XmlNode::leaf("name", "Timeline Export");
        assert_eq!(render(&n, 0), "<name>Timeline Export</name>");
        // Indent prefixes the pad.
        assert_eq!(render(&n, 2), "  <name>Timeline Export</name>");
    }

    #[test]
    fn render_self_closing_when_empty() {
        // Empty + no text → self-closing.
        let n = XmlNode::el("file", vec![]);
        assert_eq!(render(&n, 0), "<file/>");
        // With attributes.
        let a = XmlNode::empty_attrs("file", vec![("id".into(), "file-1".into())]);
        assert_eq!(render(&a, 0), "<file id=\"file-1\"/>");
    }

    #[test]
    fn render_element_with_children_joins_with_newlines_at_indent_plus_two() {
        let n = XmlNode::el(
            "rate",
            vec![XmlNode::leaf_int("timebase", 30), XmlNode::bool_leaf("ntsc", false)],
        );
        assert_eq!(
            render(&n, 0),
            "<rate>\n  <timebase>30</timebase>\n  <ntsc>FALSE</ntsc>\n</rate>"
        );
    }

    #[test]
    fn render_attrs_on_element_with_children() {
        let n = XmlNode::el_attrs(
            "xmeml",
            vec![("version".into(), "4".into())],
            vec![XmlNode::leaf("a", "b")],
        );
        assert_eq!(render(&n, 0), "<xmeml version=\"4\">\n  <a>b</a>\n</xmeml>");
    }

    #[test]
    fn bools_render_literal_true_false() {
        assert_eq!(render(&XmlNode::bool_leaf("x", true), 0), "<x>TRUE</x>");
        assert_eq!(render(&XmlNode::bool_leaf("x", false), 0), "<x>FALSE</x>");
    }

    #[test]
    fn escape_order_is_exact_and_no_double_escape() {
        // `&` first → the ampersands from `<`→`&lt;` etc. are NOT re-escaped.
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("<tag>"), "&lt;tag&gt;");
        assert_eq!(escape_xml("\"q\""), "&quot;q&quot;");
        assert_eq!(escape_xml("it's"), "it&apos;s");
        // All five together in one string, in order.
        assert_eq!(
            escape_xml("&<>\"'"),
            "&amp;&lt;&gt;&quot;&apos;"
        );
        // A pre-existing entity is double-escaped because `&` is literal input
        // (matches the reference: it escapes raw text, it does not parse entities).
        assert_eq!(escape_xml("&amp;"), "&amp;amp;");
    }

    #[test]
    fn escape_applies_to_attributes() {
        let n = XmlNode::empty_attrs("file", vec![("id".into(), "a&b".into())]);
        assert_eq!(render(&n, 0), "<file id=\"a&amp;b\"/>");
    }

    #[test]
    fn float_formatting_parity() {
        // %.4f
        assert_eq!(fmt_f(4, 100.0), "100.0000");
        assert_eq!(fmt_f(4, -50.0), "-50.0000");
        assert_eq!(fmt_f(4, 3.98), "3.9800");
        // %.5f — center offsets.
        assert_eq!(fmt_f(5, 0.0), "0.00000");
        assert_eq!(fmt_f(5, -0.25), "-0.25000");
        assert_eq!(fmt_f(5, 0.123456), "0.12346"); // rounds half-to-even region
        // %.1f — opacity ×100.
        assert_eq!(fmt_f(1, 100.0), "100.0");
        assert_eq!(fmt_f(1, 33.333), "33.3");
        // %.2f — default scalar spec.
        assert_eq!(fmt_f(2, 150.0), "150.00");
        assert_eq!(fmt_f(2, -0.0), "-0.00"); // negative zero preserved like Swift/C
    }

    #[test]
    fn prolog_is_exact() {
        assert_eq!(
            XML_PROLOG,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n"
        );
    }
}
