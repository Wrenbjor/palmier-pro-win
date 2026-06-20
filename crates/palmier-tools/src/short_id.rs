//! ShortId — UUID prefix shortening on outputs, prefix expansion on inputs
//! (reference `ToolExecutor+ShortId.swift`).
//!
//! Entity ids are full UUIDs. Sent verbatim they cost ~36 chars each and dominate
//! large `get_timeline` / `get_transcript` payloads. We emit the shortest prefix
//! that's unique within the project and accept any prefix back: tools always run
//! on full ids (resolved on input), and every text response has its known ids
//! shortened on the way out.
//!
//! ## One snapshot per call
//! [`IdUniverse`] is built **once per tool call** ([`IdUniverse::from_library`]):
//! all track ids, clip ids, `caption_group_id`, `link_group_id`, asset ids, and
//! folder ids in one `HashSet<String>` (reference `currentIdUniverse`). A
//! min-unique prefix is distinct across the whole set, so anything we emit
//! resolves to exactly one id and any unambiguous prefix we accept maps back.
//!
//! ## Input expansion runs on a fixed key allowlist
//! [`expand_id_prefixes`] only expands keys in [`SCALAR_ID_KEYS`] /
//! [`ARRAY_ID_KEYS`], recursing into nested objects/arrays. Exact match → keep;
//! exactly one prefix match → expand; **more than one → ambiguity error**; zero →
//! pass through (the tool emits its own not-found).
//!
//! ## Carry-forward gotcha (do not remove)
//! A **new id-bearing input field must be added to one of these two allowlists**
//! or the dispatcher won't accept prefixes for it. The reference's `scalarIdKeys`
//! / `arrayIdKeys` are the parity source — keep these in lockstep with any new
//! tool field that names an id.

use std::collections::HashSet;
use std::sync::LazyLock;

use palmier_model::MediaLibrary;
use regex::Regex;
use serde_json::Value;

/// Minimum id-prefix length emitted on output (reference `idPrefixFloor = 8`).
pub const ID_PREFIX_FLOOR: usize = 8;

/// Scalar (single-id) argument keys eligible for prefix expansion
/// (reference `scalarIdKeys`). Wire keys are camelCase to match the tool schemas.
///
/// **Carry-forward:** a new single-id field must be added here (see module docs).
pub const SCALAR_ID_KEYS: &[&str] = &[
    "clipId",
    "sourceClipId",
    "mediaRef",
    "startFrameMediaRef",
    "endFrameMediaRef",
    "sourceVideoMediaRef",
    "videoSourceMediaRef",
    "folderId",
    "parentFolderId",
];

/// Array-of-id argument keys eligible for prefix expansion
/// (reference `arrayIdKeys`).
///
/// **Carry-forward:** a new id-array field must be added here (see module docs).
pub const ARRAY_ID_KEYS: &[&str] = &[
    "clipIds",
    "assetIds",
    "folderIds",
    "referenceMediaRefs",
    "referenceImageMediaRefs",
    "referenceVideoMediaRefs",
    "referenceAudioMediaRefs",
];

/// Canonical hyphenated-UUID pattern (reference `uuidRegex` literal). Used to find
/// every known UUID in result text and rewrite it to its short prefix; unknown
/// UUIDs (e.g. ones embedded in a filename) aren't in the map and pass through.
static UUID_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}")
        .expect("static UUID regex is valid")
});

/// Raised when an input id-prefix is ambiguous (matches >1 universe id). The
/// dispatcher wraps this into the `{ isError: true }` tool-result shape
/// (reference `ToolError("Ambiguous id …")`).
#[derive(Debug, Clone, PartialEq)]
pub struct AmbiguousIdError {
    pub message: String,
}

impl std::fmt::Display for AmbiguousIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AmbiguousIdError {}

/// Every entity id the agent can see or name back, snapshotted once per tool call.
///
/// One set serves both directions: a min-unique prefix is distinct across the
/// whole set (reference `currentIdUniverse`).
#[derive(Debug, Clone, Default)]
pub struct IdUniverse {
    ids: HashSet<String>,
}

impl IdUniverse {
    /// Build the universe from an explicit id collection (used in tests and by
    /// callers that don't hold a full [`MediaLibrary`]).
    pub fn from_ids<I, S>(ids: I) -> IdUniverse
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        IdUniverse { ids: ids.into_iter().map(Into::into).collect() }
    }

    /// Snapshot all ids from a [`MediaLibrary`]: every track id, clip id,
    /// `caption_group_id`, `link_group_id`, asset id, and folder id
    /// (reference `currentIdUniverse(editor)`).
    pub fn from_library(library: &MediaLibrary) -> IdUniverse {
        let mut ids = HashSet::new();
        for track in &library.timeline.tracks {
            ids.insert(track.id.clone());
            for clip in &track.clips {
                ids.insert(clip.id.clone());
                if let Some(g) = &clip.caption_group_id {
                    ids.insert(g.clone());
                }
                if let Some(g) = &clip.link_group_id {
                    ids.insert(g.clone());
                }
            }
        }
        for asset in &library.assets {
            ids.insert(asset.id.clone());
        }
        for folder in &library.manifest.folders {
            ids.insert(folder.id.clone());
        }
        IdUniverse { ids }
    }

    /// Whether the universe contains `id` exactly.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Number of ids in the universe.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether the universe is empty.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The shortest prefix (≥ [`ID_PREFIX_FLOOR`] chars) of `id` that no other id
    /// in the universe shares. Returns the full id if even that isn't unique
    /// (reference `shortIdMap` inner loop). `id` is assumed to be in the universe.
    fn short_prefix(&self, id: &str) -> String {
        // Operate on chars to stay correct for any UTF-8 (UUIDs are ASCII, but the
        // reference uses `id.prefix(len)` which is char-based).
        let chars: Vec<char> = id.chars().collect();
        let mut len = ID_PREFIX_FLOOR.min(chars.len());
        loop {
            let prefix: String = chars[..len].iter().collect();
            let collides = self
                .ids
                .iter()
                .any(|other| other != id && other.starts_with(&prefix));
            if !collides || len >= chars.len() {
                return prefix;
            }
            len += 1;
        }
    }

    /// Rewrite every **known** UUID in `text` to its short prefix. Unknown UUIDs
    /// (not in the universe) pass through untouched (reference `shorteningIds`).
    pub fn shorten_text(&self, text: &str) -> String {
        if self.ids.is_empty() {
            return text.to_string();
        }
        UUID_REGEX
            .replace_all(text, |caps: &regex::Captures<'_>| {
                let matched = &caps[0];
                if self.ids.contains(matched) {
                    self.short_prefix(matched)
                } else {
                    matched.to_string()
                }
            })
            .into_owned()
    }

    /// Resolve a single id-prefix to a full id (reference `expandOne`):
    /// exact match → keep; exactly one prefix match → expand; >1 → ambiguity
    /// error; 0 → pass `ref_str` through unchanged (the tool emits its not-found).
    pub fn expand_one(&self, ref_str: &str) -> Result<String, AmbiguousIdError> {
        if self.ids.contains(ref_str) {
            return Ok(ref_str.to_string());
        }
        let matches: Vec<&String> =
            self.ids.iter().filter(|id| id.starts_with(ref_str)).collect();
        match matches.len() {
            1 => Ok(matches[0].clone()),
            0 => Ok(ref_str.to_string()),
            n => Err(AmbiguousIdError {
                message: format!(
                    "Ambiguous id '{ref_str}' matches {n} items; re-read with get_timeline or get_media for current ids."
                ),
            }),
        }
    }
}

/// Expand id-prefix arguments back to full ids before a tool runs (reference
/// `expandingIdPrefixes` → `expand`). Only keys in [`SCALAR_ID_KEYS`] /
/// [`ARRAY_ID_KEYS`] are expanded; recursion descends into nested objects/arrays.
/// Returns an [`AmbiguousIdError`] on the first ambiguous prefix.
pub fn expand_id_prefixes(args: &Value, universe: &IdUniverse) -> Result<Value, AmbiguousIdError> {
    expand(args, universe)
}

fn expand(value: &Value, universe: &IdUniverse) -> Result<Value, AmbiguousIdError> {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, v) in map {
                let expanded = if SCALAR_ID_KEYS.contains(&key.as_str()) {
                    // Scalar id key: expand only if it's a string; else recurse.
                    match v {
                        Value::String(s) => Value::String(universe.expand_one(s)?),
                        other => expand(other, universe)?,
                    }
                } else if ARRAY_ID_KEYS.contains(&key.as_str()) {
                    // Array-of-id key: expand each string element; non-strings kept.
                    match v {
                        Value::Array(arr) => {
                            let mut out_arr = Vec::with_capacity(arr.len());
                            for el in arr {
                                let e = match el {
                                    Value::String(s) => {
                                        Value::String(universe.expand_one(s)?)
                                    }
                                    other => other.clone(),
                                };
                                out_arr.push(e);
                            }
                            Value::Array(out_arr)
                        }
                        other => expand(other, universe)?,
                    }
                } else {
                    expand(v, universe)?
                };
                out.insert(key.clone(), expanded);
            }
            Ok(Value::Object(out))
        }
        Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for el in arr {
                out.push(expand(el, universe)?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other.clone()),
    }
}
