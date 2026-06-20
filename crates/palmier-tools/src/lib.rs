//! # palmier-tools
//!
//! Shared tool dispatch — the single implementation of the 30 MCP tools, invoked
//! by BOTH the MCP server (`palmier-mcp`) and the in-app agent (`palmier-agent`),
//! exactly one impl per tool name (FOUNDATION §4, §6.14). Operates over the
//! `palmier-model` shapes; ID prefix shortening + agent undo stack live here.
//!
//! ## This story (E7-S1) — the dispatch scaffold
//!
//! The foundation of the product's strategic centerpiece (the MCP server). It
//! builds the **registry + types + dispatch seam + ShortId**, NOT the 30 tool
//! bodies (those are E7-S5..S10):
//!
//! - [`schema`] — the [`ToolName`] enum (exactly **30** wire names), the
//!   [`ToolDefinition`] struct + [`tool_definitions`] registry carrying the
//!   **verbatim** reference descriptions + JSON input schemas, and the
//!   [`object_schema`] helper (empty props/required omitted, per the reference).
//! - [`result`] — [`ToolResult`] + [`Block`] and the contract error shape
//!   `{ "isError": true, "content": [{ "type": "text", "text": … }] }`.
//! - [`dispatch`] — the [`ToolDispatch`] trait (the single seam MCP + agent call)
//!   and the [`ScaffoldDispatcher`]: name resolution → ShortId expand → exhaustive
//!   30-arm `run` → ShortId shorten → error wrapping. Bodies return a structured
//!   "not yet implemented" result for now.
//! - [`short_id`] — [`IdUniverse`] snapshot, ≥8-char min-unique-prefix output
//!   shortening, key-allowlist input expansion with ambiguity → tool error.
//! - [`resources`] — the two `palmier://models/{video,image}` resource descriptors
//!   (resources, NOT tools — they don't count toward the 30; SM-C2).
//!
//! Parity authority: the macOS reference `Sources/PalmierPro/Agent/Tools/*`
//! (`ToolDefinitions.swift`, `ToolExecutor.swift`, `ToolExecutor+ShortId.swift`,
//! `ToolResult.swift`) and `docs/reference/mcp-tools.md`. Tool descriptions and the
//! 30-count are load-bearing contract — see [`schema`] (ruling #1, #2; R-5).

pub mod clips;
pub mod dispatch;
pub mod editor;
pub mod executor;
pub mod generate;
pub mod inspect;
pub mod json_round;
pub mod library;
pub mod properties;
pub mod read;
pub mod resources;
pub mod result;
pub mod schema;
pub mod short_id;
pub mod texts;
pub mod transcript;
pub mod undo;
pub mod validate;

pub use dispatch::{ScaffoldDispatcher, ToolContext, ToolDispatch};
pub use editor::{AgentStack, EditorState};
pub use generate::{GenerationGateway, GenerationSubmission, BACKEND_NOT_AVAILABLE};
pub use executor::ToolExecutor;
pub use json_round::{round_json_numbers, JSON_ROUND_PLACES};
pub use read::{CAPTION_ROW_LIMIT, TRANSCRIPT_WORD_CAP};
pub use resources::{
    ResourceDescriptor, IMAGE_MODELS_RESOURCE, RESOURCE_DESCRIPTORS, VIDEO_MODELS_RESOURCE,
};
pub use result::{Block, ToolResult};
pub use schema::{object_schema, tool_definitions, ToolDefinition, ToolName};
pub use short_id::{
    expand_id_prefixes, AmbiguousIdError, IdUniverse, ARRAY_ID_KEYS, ID_PREFIX_FLOOR,
    SCALAR_ID_KEYS,
};
pub use transcript::span_frames;
pub use validate::{parse_rgba, validate, ValidationError};

/// The count of MCP tools the surface exposes. **Exactly 30** (ruling #1 —
/// `§13.12` void; there is no missing-6 set). Asserted against
/// [`ToolName::ALL`] and [`tool_definitions`] in the test suite. SM-C2: a 31st
/// tool must fail the count gate.
pub const TOOL_COUNT: usize = 30;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// A test [`ToolContext`] backed by an explicit id set.
    struct TestCtx {
        universe: IdUniverse,
    }
    impl TestCtx {
        fn empty() -> TestCtx {
            TestCtx { universe: IdUniverse::default() }
        }
        fn with_ids<const N: usize>(ids: [&str; N]) -> TestCtx {
            TestCtx { universe: IdUniverse::from_ids(ids) }
        }
    }
    impl ToolContext for TestCtx {
        fn id_universe(&self) -> IdUniverse {
            self.universe.clone()
        }
    }

    // ── count gates (SM-C2, ruling #1) ──────────────────────────────────────

    #[test]
    fn tool_name_has_exactly_30_variants() {
        assert_eq!(ToolName::ALL.len(), 30);
        assert_eq!(ToolName::ALL.len(), TOOL_COUNT);
    }

    #[test]
    fn registry_has_exactly_30_definitions() {
        assert_eq!(tool_definitions().len(), 30);
        assert_eq!(tool_definitions().len(), TOOL_COUNT);
    }

    #[test]
    fn all_30_wire_names_are_unique() {
        let mut names: Vec<&str> = ToolName::ALL.iter().map(|t| t.wire_name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate wire name(s) found");
    }

    /// The exact 30 wire names from the story acceptance criteria, as a set.
    #[test]
    fn wire_names_match_the_reference_catalogue() {
        let expected = [
            "get_timeline", "get_media", "inspect_media", "get_transcript", "inspect_timeline",
            "search_media", "list_models", "list_folders", "add_clips", "remove_clips",
            "remove_tracks", "move_clips", "set_clip_properties", "set_keyframes", "split_clip",
            "ripple_delete_ranges", "undo", "add_texts", "add_captions", "generate_video",
            "generate_image", "generate_audio", "upscale_media", "import_media", "create_folder",
            "move_to_folder", "rename_media", "rename_folder", "delete_media", "delete_folder",
        ];
        let mut got: Vec<&str> = ToolName::ALL.iter().map(|t| t.wire_name()).collect();
        got.sort_unstable();
        let mut want: Vec<&str> = expected.to_vec();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    // ── ToolName round-trip (all 30) ────────────────────────────────────────

    #[test]
    fn every_tool_name_round_trips_through_wire() {
        for tool in ToolName::ALL {
            let wire = tool.wire_name();
            let back = ToolName::from_wire(wire);
            assert_eq!(back, Some(tool), "round-trip failed for {wire}");
        }
    }

    #[test]
    fn unknown_wire_name_resolves_to_none() {
        assert_eq!(ToolName::from_wire("not_a_tool"), None);
        assert_eq!(ToolName::from_wire(""), None);
    }

    // ── registry / schema integrity ─────────────────────────────────────────

    #[test]
    fn registry_covers_every_tool_name_once() {
        let defs = tool_definitions();
        for tool in ToolName::ALL {
            let matches = defs.iter().filter(|d| d.name == tool).count();
            assert_eq!(matches, 1, "expected exactly one definition for {:?}", tool);
        }
    }

    #[test]
    fn every_description_is_non_empty() {
        for def in tool_definitions() {
            assert!(
                !def.description.is_empty(),
                "empty description for {:?}",
                def.name
            );
        }
    }

    /// Each input schema is a JSON-Schema object and round-trips through serde
    /// unchanged (the rmcp / serde_json round-trip the story requires).
    #[test]
    fn every_input_schema_is_an_object_and_round_trips() {
        for def in tool_definitions() {
            let schema = &def.input_schema;
            assert_eq!(
                schema.get("type").and_then(Value::as_str),
                Some("object"),
                "schema for {:?} is not type=object",
                def.name
            );
            let text = serde_json::to_string(schema).expect("serialize schema");
            let back: Value = serde_json::from_str(&text).expect("deserialize schema");
            assert_eq!(&back, schema, "schema round-trip changed {:?}", def.name);
        }
    }

    #[test]
    fn mutation_and_async_flags_match_classification() {
        for def in tool_definitions() {
            assert_eq!(def.mutation, def.name.is_mutation());
            assert_eq!(def.is_async, def.name.is_async());
        }
        // Spot-check the load-bearing classifications (mcp-tools.md table):
        assert!(!ToolName::GetTimeline.is_mutation());
        assert!(ToolName::AddClips.is_mutation());
        assert!(ToolName::GenerateVideo.is_mutation() && ToolName::GenerateVideo.is_async());
        assert!(ToolName::InspectMedia.is_async() && !ToolName::InspectMedia.is_mutation());
    }

    // ── object_schema helper (empty omission parity) ────────────────────────

    #[test]
    fn object_schema_omits_empty_properties_and_required() {
        let empty = object_schema(&[], &[]);
        assert_eq!(empty, json!({ "type": "object" }));
        assert!(empty.get("properties").is_none());
        assert!(empty.get("required").is_none());
    }

    #[test]
    fn object_schema_includes_non_empty_properties_and_required() {
        let s = object_schema(
            &[("foo", json!({ "type": "string" }))],
            &["foo"],
        );
        assert_eq!(s["properties"]["foo"]["type"], json!("string"));
        assert_eq!(s["required"], json!(["foo"]));
    }

    /// Pin the easy-to-miss schema cases the story calls out.
    #[test]
    fn generate_audio_has_no_required_field() {
        let def = tool_definitions()
            .into_iter()
            .find(|d| d.name == ToolName::GenerateAudio)
            .unwrap();
        assert!(
            def.input_schema.get("required").is_none(),
            "generate_audio must have NO required field (prompt optional, video-to-music)"
        );
    }

    #[test]
    fn dual_shape_tools_have_no_required_and_carry_entries() {
        for tool in [
            ToolName::CreateFolder,
            ToolName::MoveToFolder,
            ToolName::RenameMedia,
            ToolName::RenameFolder,
        ] {
            let def = tool_definitions().into_iter().find(|d| d.name == tool).unwrap();
            // Dual-shape: direct fields XOR entries[]; neither alone is `required`.
            assert!(
                def.input_schema.get("required").is_none(),
                "{:?} must not pin a required field (direct XOR entries)",
                tool
            );
            assert!(
                def.input_schema["properties"].get("entries").is_some(),
                "{:?} must expose the entries[] alternate shape",
                tool
            );
        }
    }

    #[test]
    fn set_keyframes_interp_enum_and_smooth_default_documented() {
        let def = tool_definitions()
            .into_iter()
            .find(|d| d.name == ToolName::SetKeyframes)
            .unwrap();
        // interp default smooth is documented in the description (ruling #8).
        assert!(def.description.contains("default smooth"));
        // property enum present and exact.
        let prop_enum = &def.input_schema["properties"]["property"]["enum"];
        assert_eq!(
            prop_enum,
            &json!(["volume", "opacity", "rotation", "position", "scale", "crop"])
        );
    }

    #[test]
    fn inspect_media_and_timeline_document_max_frames_cap() {
        for tool in [ToolName::InspectMedia, ToolName::InspectTimeline] {
            let def = tool_definitions().into_iter().find(|d| d.name == tool).unwrap();
            assert!(
                def.input_schema["properties"]["maxFrames"]["description"]
                    .as_str()
                    .unwrap()
                    .contains("max 12"),
                "{:?} must document the max_frames<=12 ceiling",
                tool
            );
        }
    }

    #[test]
    fn ripple_delete_requires_only_ranges() {
        let def = tool_definitions()
            .into_iter()
            .find(|d| d.name == ToolName::RippleDeleteRanges)
            .unwrap();
        assert_eq!(def.input_schema["required"], json!(["ranges"]));
        // trackIndex XOR clipId are described, not pinned as required.
        assert!(def.input_schema["properties"].get("trackIndex").is_some());
        assert!(def.input_schema["properties"].get("clipId").is_some());
    }

    /// Verbatim-preservation spot check: the Unicode glyphs must survive as UTF-8.
    #[test]
    fn descriptions_preserve_unicode_glyphs() {
        let set_kf = tool_definitions()
            .into_iter()
            .find(|d| d.name == ToolName::SetKeyframes)
            .unwrap();
        assert!(set_kf.description.contains('•'), "bullet U+2022 preserved");
        assert!(set_kf.description.contains('∈'), "element-of preserved");
        assert!(set_kf.description.contains('–'), "en-dash U+2013 preserved");

        // The × (U+00D7) and − (U+2212) glyphs live in set_clip_properties'
        // trimStartFrame *property* description inside the input schema.
        let scp = tool_definitions()
            .into_iter()
            .find(|d| d.name == ToolName::SetClipProperties)
            .unwrap();
        let trim_desc = scp.input_schema["properties"]["trimStartFrame"]["description"]
            .as_str()
            .unwrap();
        assert!(trim_desc.contains('×'), "multiplication sign U+00D7 preserved");
        assert!(trim_desc.contains('−'), "minus sign U+2212 preserved");
    }

    // ── ToolResult error/ok shape ───────────────────────────────────────────

    #[test]
    fn error_result_matches_contract_shape() {
        let r = ToolResult::error("boom");
        assert!(r.is_error);
        let wire = r.to_mcp_json();
        assert_eq!(
            wire,
            json!({ "isError": true, "content": [{ "type": "text", "text": "boom" }] })
        );
    }

    #[test]
    fn ok_result_omits_is_error_key() {
        let r = ToolResult::ok("hi");
        let wire = r.to_mcp_json();
        assert_eq!(wire, json!({ "content": [{ "type": "text", "text": "hi" }] }));
        assert!(wire.get("isError").is_none());
    }

    #[test]
    fn image_block_maps_to_data_and_mime() {
        let r = ToolResult {
            content: vec![Block::Image { base64: "Zm9v".into(), media_type: "image/png".into() }],
            is_error: false,
        };
        let wire = r.to_mcp_json();
        assert_eq!(
            wire["content"][0],
            json!({ "type": "image", "data": "Zm9v", "mimeType": "image/png" })
        );
    }

    // ── ShortId: shorten / expand / ambiguity ───────────────────────────────

    // Two ids sharing an 8-char prefix; differ at char 9 → forces a ≥9 shorten.
    const ID_A: &str = "aaaaaaaa-1111-1111-1111-111111111111";
    const ID_B: &str = "aaaaaaaa-2222-2222-2222-222222222222";
    // A distinct id sharing nothing in the first 8.
    const ID_C: &str = "bcdef012-3333-3333-3333-333333333333";

    #[test]
    fn shorten_uses_eight_char_floor_when_unique() {
        let u = IdUniverse::from_ids([ID_C]);
        let text = format!("clip {ID_C} added");
        // Unique at the floor → exactly 8 chars.
        assert_eq!(u.shorten_text(&text), "clip bcdef012 added");
    }

    #[test]
    fn shorten_extends_prefix_to_break_a_collision() {
        let u = IdUniverse::from_ids([ID_A, ID_B]);
        // ID_A/ID_B share "aaaaaaaa-" (9 chars incl. the hyphen) and first differ
        // at char 10 ('1' vs '2'), so the min-unique prefix is 10 chars.
        let out = u.shorten_text(ID_A);
        assert_eq!(out, "aaaaaaaa-1");
        assert!(out.len() > ID_PREFIX_FLOOR, "collision forced a prefix past the floor");
    }

    #[test]
    fn shorten_leaves_unknown_uuids_untouched() {
        let u = IdUniverse::from_ids([ID_C]);
        // A UUID embedded in a filename that is NOT in the universe.
        let unknown = "deadbeef-0000-0000-0000-000000000000";
        let text = format!("imported file_{unknown}.mp4");
        assert_eq!(u.shorten_text(&text), text, "unknown UUID must pass through");
    }

    #[test]
    fn expand_exact_match_keeps_the_id() {
        let u = IdUniverse::from_ids([ID_C]);
        assert_eq!(u.expand_one(ID_C).unwrap(), ID_C);
    }

    #[test]
    fn expand_unique_prefix_resolves_to_full_id() {
        let u = IdUniverse::from_ids([ID_C]);
        assert_eq!(u.expand_one("bcdef012").unwrap(), ID_C);
    }

    #[test]
    fn expand_ambiguous_prefix_is_a_tool_error() {
        let u = IdUniverse::from_ids([ID_A, ID_B]);
        let err = u.expand_one("aaaaaaaa").unwrap_err();
        assert!(err.message.contains("Ambiguous id 'aaaaaaaa'"));
        assert!(err.message.contains("2 items"));
    }

    #[test]
    fn expand_below_floor_or_nonexistent_passes_through() {
        let u = IdUniverse::from_ids([ID_C]);
        // Non-existent prefix → passed through (tool emits its own not-found).
        assert_eq!(u.expand_one("zzz").unwrap(), "zzz");
    }

    #[test]
    fn expand_only_touches_allowlisted_keys() {
        let u = IdUniverse::from_ids([ID_C]);
        let args = json!({
            "clipId": "bcdef012",                 // scalar allowlist → expand
            "clipIds": ["bcdef012"],              // array allowlist → expand
            "notAnId": "bcdef012",                // not allowlisted → untouched
            "nested": { "mediaRef": "bcdef012" }, // recurse into objects
        });
        let out = expand_id_prefixes(&args, &u).unwrap();
        assert_eq!(out["clipId"], json!(ID_C));
        assert_eq!(out["clipIds"], json!([ID_C]));
        assert_eq!(out["notAnId"], json!("bcdef012"), "non-allowlisted key untouched");
        assert_eq!(out["nested"]["mediaRef"], json!(ID_C), "recursion into nested objects");
    }

    #[test]
    fn expand_propagates_ambiguity_from_nested_array() {
        let u = IdUniverse::from_ids([ID_A, ID_B]);
        let args = json!({ "clipIds": ["aaaaaaaa"] });
        let err = expand_id_prefixes(&args, &u).unwrap_err();
        assert!(err.message.contains("Ambiguous id"));
    }

    // ── dispatch seam ───────────────────────────────────────────────────────

    #[test]
    fn dispatch_routes_every_one_of_the_30_names() {
        let d = ScaffoldDispatcher::new();
        let ctx = TestCtx::empty();
        for tool in ToolName::ALL {
            let r = d.execute(tool.wire_name(), json!({}), &ctx);
            // A routed (stubbed) tool is NOT the error shape — it reached its arm.
            assert!(
                !r.is_error,
                "{} should route to its arm, not error",
                tool.wire_name()
            );
            match &r.content[0] {
                Block::Text(s) => assert!(
                    s.contains(tool.wire_name()),
                    "stub result for {} should name the tool",
                    tool.wire_name()
                ),
                _ => panic!("expected a text block"),
            }
        }
    }

    #[test]
    fn dispatch_unknown_name_returns_error_shape() {
        let d = ScaffoldDispatcher::new();
        let ctx = TestCtx::empty();
        let r = d.execute("not_a_tool", json!({}), &ctx);
        assert!(r.is_error);
        let wire = r.to_mcp_json();
        assert_eq!(wire["isError"], json!(true));
        assert!(wire["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Unknown tool: not_a_tool"));
    }

    #[test]
    fn dispatch_wraps_ambiguous_input_prefix_as_tool_error() {
        let d = ScaffoldDispatcher::new();
        let ctx = TestCtx::with_ids([ID_A, ID_B]);
        // clipId is an allowlisted scalar id; "aaaaaaaa" is ambiguous.
        let r = d.execute("split_clip", json!({ "clipId": "aaaaaaaa" }), &ctx);
        assert!(r.is_error, "ambiguous input prefix must wrap into the error shape");
        let wire = r.to_mcp_json();
        assert_eq!(wire["isError"], json!(true));
        assert!(wire["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Ambiguous id"));
    }

    // ── resources (2, not tools) ────────────────────────────────────────────

    #[test]
    fn exactly_two_resource_descriptors() {
        assert_eq!(RESOURCE_DESCRIPTORS.len(), 2);
        assert_eq!(VIDEO_MODELS_RESOURCE.uri, "palmier://models/video");
        assert_eq!(IMAGE_MODELS_RESOURCE.uri, "palmier://models/image");
        for r in RESOURCE_DESCRIPTORS {
            assert_eq!(r.mime_type, "application/json");
        }
    }
}
