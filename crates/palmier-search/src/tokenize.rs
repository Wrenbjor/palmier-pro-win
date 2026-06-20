//! Text tokenization — reproduce the reference's pad-to-64, id 0, no-mask scheme.
//!
//! Ported from Spike S-3 (`spikes/s3-siglip2/src/tokenize.rs`), proven against the
//! macOS reference `TextTokenizer.swift`.
//!
//! Reference (`TextTokenizer.swift`):
//! ```text
//! var ids = tokenizer.encode(text).map(Int32.init)
//! if ids.count > contextLength { ids = ids.prefix(contextLength) }   // truncate
//! ids += repeat(padToken=0, contextLength - ids.count)               // right-pad 0
//! return ids                                                          // no mask
//! ```
//! The reference loads the model via HuggingFace `AutoTokenizer.from(modelFolder:)`,
//! i.e. it reads the very same `tokenizer.json` we load here with the `tokenizers`
//! crate. SigLIP/Gemma config: `do_lower_case=true`, `add_eos_token=true`,
//! `add_bos_token=false`, pad id 0. The `tokenizer.json` already encodes the
//! normalizer (lowercasing) and the post-processor (eos append), so calling
//! `encode(text, add_special_tokens=true)` matches the reference's AutoTokenizer.
//!
//! The ONNX `text_model.onnx` takes `input_ids [1,64] i64` — no attention mask,
//! exactly as SigLIP was trained (fixed max_length padding). We emit i64 here.

use anyhow::{Context, Result};
use std::path::Path;
use tokenizers::Tokenizer;

use crate::spec::CONTEXT_LENGTH;

/// Wraps a loaded HF tokenizer and applies the reference pad/truncate rule.
pub struct SiglipTokenizer {
    inner: Tokenizer,
    context_length: usize,
}

impl SiglipTokenizer {
    /// Load from a `tokenizer.json` (the file the reference's AutoTokenizer reads).
    pub fn from_file(path: &Path) -> Result<Self> {
        let inner = Tokenizer::from_file(path)
            .map_err(|e| anyhow::anyhow!("load tokenizer.json: {e}"))
            .with_context(|| format!("path: {}", path.display()))?;
        Ok(Self { inner, context_length: CONTEXT_LENGTH })
    }

    /// For tests / non-default context lengths: build from an in-memory tokenizer.
    pub fn from_tokenizer(inner: Tokenizer, context_length: usize) -> Self {
        Self { inner, context_length }
    }

    /// The context length this tokenizer pads/truncates to.
    pub fn context_length(&self) -> usize {
        self.context_length
    }

    /// Tokenize → truncate to contextLength → right-pad with id 0. Returns the
    /// fixed-length `i64` ids the ONNX `input_ids` input wants. No attention mask.
    ///
    /// `add_special_tokens=true` so the tokenizer.json post-processor appends `<eos>`
    /// (add_eos_token=true) exactly as the reference AutoTokenizer does. Lowercasing
    /// is handled by the tokenizer.json normalizer (do_lower_case=true).
    pub fn encode(&self, text: &str) -> Result<Vec<i64>> {
        let enc = self
            .inner
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let mut ids: Vec<i64> = enc.get_ids().iter().map(|&u| u as i64).collect();
        if ids.len() > self.context_length {
            ids.truncate(self.context_length); // reference: prefix(contextLength)
        }
        ids.resize(self.context_length, 0); // reference: right-pad with padToken=0
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny whitespace tokenizer with a fixed vocab, so the pad/truncate logic is
    /// testable without downloading the 34 MB Gemma tokenizer.json. (The real
    /// tokenizer.json equivalence is asserted in the `--features ort` live path.)
    fn toy(context_length: usize) -> SiglipTokenizer {
        use std::collections::HashMap;
        use tokenizers::models::wordlevel::WordLevel;
        use tokenizers::pre_tokenizers::whitespace::Whitespace;

        let mut vocab: HashMap<String, u32> = HashMap::new();
        // id 0 reserved as the pad token (never emitted by encode of real words).
        vocab.insert("<pad>".into(), 0);
        for (i, w) in ["a", "quick", "brown", "fox", "jumps"].iter().enumerate() {
            vocab.insert((*w).into(), (i + 1) as u32);
        }
        let wl = WordLevel::builder()
            .vocab(vocab)
            .unk_token("<pad>".into())
            .build()
            .unwrap();
        let mut tk = Tokenizer::new(wl);
        tk.with_pre_tokenizer(Some(Whitespace {}));
        SiglipTokenizer::from_tokenizer(tk, context_length)
    }

    #[test]
    fn pads_to_context_length_with_zero() {
        let t = toy(64);
        let ids = t.encode("quick brown fox").unwrap();
        assert_eq!(ids.len(), 64);
        assert_eq!(&ids[..3], &[2, 3, 4]); // quick, brown, fox
        assert!(ids[3..].iter().all(|&x| x == 0), "tail must be pad id 0");
    }

    #[test]
    fn truncates_to_context_length() {
        let t = toy(2);
        let ids = t.encode("quick brown fox jumps").unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids, vec![2, 3]); // first 2 only
    }

    #[test]
    fn empty_text_is_all_pad() {
        let t = toy(8);
        let ids = t.encode("").unwrap();
        assert_eq!(ids, vec![0; 8]);
    }
}
