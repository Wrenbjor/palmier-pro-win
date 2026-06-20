//! MCP resource descriptors — the two `palmier://models/*` resources as data
//! (reference `MCPService.swift:96-133`).
//!
//! These are **resources, not tools** — they do NOT count toward the 30 (SM-C2)
//! and must not be registered as tools. The `palmier-mcp` transport (E7-S11/S13)
//! registers them with `listChanged: false`, `subscribe: false`. The resource
//! *bodies* (the JSON model arrays from `videoModelInfo` / `imageModelInfo`) come
//! from the generation catalog, wired by Epic 9 (M3); until then clients tolerate
//! an empty array. This scaffold provides the **descriptors** (uri/name/mime).

/// A static MCP resource descriptor (reference `Resource(name:uri:description:mimeType:)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceDescriptor {
    /// Human-readable name (reference `name`).
    pub name: &'static str,
    /// The `palmier://…` URI (reference `uri`).
    pub uri: &'static str,
    /// Description string (reference `description`).
    pub description: &'static str,
    /// MIME type — always `application/json` for these two (reference `mimeType`).
    pub mime_type: &'static str,
}

/// `palmier://models/video` — available AI video generation models.
pub const VIDEO_MODELS_RESOURCE: ResourceDescriptor = ResourceDescriptor {
    name: "Video Models",
    uri: "palmier://models/video",
    description: "Available AI video generation models and their capabilities",
    mime_type: "application/json",
};

/// `palmier://models/image` — available AI image generation models.
pub const IMAGE_MODELS_RESOURCE: ResourceDescriptor = ResourceDescriptor {
    name: "Image Models",
    uri: "palmier://models/image",
    description: "Available AI image generation models and their capabilities",
    mime_type: "application/json",
};

/// The two resource descriptors, in reference registration order. **Exactly 2** —
/// these are the complete resource surface alongside the 30 tools.
pub const RESOURCE_DESCRIPTORS: [ResourceDescriptor; 2] =
    [VIDEO_MODELS_RESOURCE, IMAGE_MODELS_RESOURCE];
