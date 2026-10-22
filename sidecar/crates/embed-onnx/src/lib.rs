//! Sentence embeddings with a sentence-transformers model exported to ONNX,
//! run on the CPU with ONNX Runtime.
//!
//! Built for `paraphrase-multilingual-MiniLM-L12-v2` (int8-quantized ONNX
//! export, about 50 languages): the model takes `input_ids`, `attention_mask`
//! and `token_type_ids` and returns `last_hidden_state`, which is mean-pooled
//! over the attention mask, as sentence-transformers does for this model.
//! Any BERT-style export with the same inputs and output works.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};
use transcribe_core::SentenceEmbedder;

/// Longest input in tokens; the model was trained with this limit.
const MAX_TOKENS: usize = 128;
/// Sentences per inference call, to bound memory on long transcripts.
const BATCH: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("paragraph model not found: {0}")]
    ModelNotFound(PathBuf),
    #[error("paragraph tokenizer not found: {0}")]
    TokenizerNotFound(PathBuf),
    #[error("failed to load paragraph tokenizer {path}: {message}")]
    TokenizerLoad { path: PathBuf, message: String },
    #[error("failed to tokenize: {0}")]
    Tokenize(String),
    #[error("ONNX Runtime error")]
    Ort(#[from] ort::Error),
    #[error("unexpected model output: {0}")]
    Output(String),
}

/// A loaded embedding model and its tokenizer.
pub struct OnnxEmbedder {
    // `Session::run` needs `&mut`; `SentenceEmbedder::embed` takes `&self`.
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl std::fmt::Debug for OnnxEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxEmbedder").finish_non_exhaustive()
    }
}

impl OnnxEmbedder {
    /// Loads the ONNX model and its Hugging Face `tokenizer.json`.
    pub fn new(
        model_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let (model_path, tokenizer_path) = (model_path.as_ref(), tokenizer_path.as_ref());
        if !model_path.is_file() {
            return Err(Error::ModelNotFound(model_path.to_path_buf()));
        }
        if !tokenizer_path.is_file() {
            return Err(Error::TokenizerNotFound(tokenizer_path.to_path_buf()));
        }

        let load_error = |message: String| Error::TokenizerLoad {
            path: tokenizer_path.to_path_buf(),
            message,
        };
        let mut tokenizer =
            Tokenizer::from_file(tokenizer_path).map_err(|e| load_error(e.to_string()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(|e| load_error(e.to_string()))?;
        tokenizer.with_padding(Some(PaddingParams::default()));

        let environment = ort::init().with_name("paragraph-model").build()?;
        let session = Session::builder(&environment)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(ort::Error::from)?
            .commit_from_file(model_path)?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    fn embed_batch(&self, sentences: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        let encodings = self
            .tokenizer
            .encode_batch(sentences.to_vec(), true)
            .map_err(|e| Error::Tokenize(e.to_string()))?;
        let rows = encodings.len();
        let cols = encodings.first().map_or(0, |e| e.get_ids().len());
        if rows == 0 || cols == 0 {
            return Ok(vec![Vec::new(); rows]);
        }

        let flat = |f: fn(&tokenizers::Encoding) -> &[u32]| -> Vec<i64> {
            encodings
                .iter()
                .flat_map(|e| f(e).iter().map(|&x| i64::from(x)))
                .collect()
        };
        let ids = flat(tokenizers::Encoding::get_ids);
        let mask = flat(tokenizers::Encoding::get_attention_mask);
        let type_ids = vec![0i64; rows * cols];
        let shape = [rows, cols];

        let mut session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let outputs = session.run(ort::inputs![
            "input_ids" => Tensor::from_array((shape, ids))?,
            "attention_mask" => Tensor::from_array((shape, mask.clone()))?,
            "token_type_ids" => Tensor::from_array((shape, type_ids))?,
        ])?;
        let hidden = outputs
            .get("last_hidden_state")
            .ok_or_else(|| Error::Output("no last_hidden_state output".to_string()))?;
        let (dims, data) = hidden.try_extract_tensor::<f32>()?;
        let dim = match **dims {
            [r, c, d] if r as usize == rows && c as usize == cols && d > 0 => d as usize,
            _ => {
                return Err(Error::Output(format!(
                    "last_hidden_state has shape {dims:?}"
                )))
            }
        };

        Ok(mean_pool(data, &mask, rows, cols, dim))
    }
}

/// Averages each row's token vectors over the tokens its mask keeps.
fn mean_pool(hidden: &[f32], mask: &[i64], rows: usize, cols: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..rows)
        .map(|r| {
            let mut sum = vec![0.0f32; dim];
            let mut kept = 0.0f32;
            for c in (0..cols).filter(|&c| mask[r * cols + c] != 0) {
                let token = &hidden[(r * cols + c) * dim..(r * cols + c + 1) * dim];
                sum.iter_mut().zip(token).for_each(|(s, x)| *s += x);
                kept += 1.0;
            }
            sum.iter().map(|s| s / kept.max(1.0)).collect()
        })
        .collect()
}

impl SentenceEmbedder for OnnxEmbedder {
    type Error = Error;

    fn embed(&self, sentences: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        sentences
            .chunks(BATCH)
            .try_fold(Vec::new(), |mut out, chunk| {
                out.extend(self.embed_batch(chunk)?);
                Ok(out)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pool_ignores_masked_tokens() {
        // Two rows of three tokens, dim 2; row 1's last token is padding.
        let hidden = [
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 1.0, 1.0, 3.0, 3.0, 100.0, 100.0,
        ];
        let mask = [1, 1, 1, 1, 1, 0];
        assert_eq!(
            mean_pool(&hidden, &mask, 2, 3, 2),
            vec![vec![3.0, 4.0], vec![2.0, 2.0]]
        );
    }

    #[test]
    fn fully_masked_row_pools_to_zero() {
        assert_eq!(mean_pool(&[5.0, 5.0], &[0], 1, 1, 2), vec![vec![0.0, 0.0]]);
    }

    #[test]
    fn missing_files_are_reported_by_name() {
        let err = OnnxEmbedder::new("/nonexistent/model.onnx", "/nonexistent/tokenizer.json")
            .unwrap_err();
        assert!(matches!(err, Error::ModelNotFound(_)), "{err}");
    }
}
