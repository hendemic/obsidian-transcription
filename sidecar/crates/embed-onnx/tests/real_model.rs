//! Slow tests with the real paragraph model. Ignored by default.
//!
//! ```sh
//! # from sidecar/
//! base=https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/main
//! curl -L -o /tmp/paragraph.onnx $base/onnx/model_qint8_arm64.onnx
//! curl -L -o /tmp/tokenizer.json $base/tokenizer.json
//! PARAGRAPH_TEST_MODEL=/tmp/paragraph.onnx PARAGRAPH_TEST_TOKENIZER=/tmp/tokenizer.json \
//!   cargo test -p embed-onnx --test real_model -- --ignored
//! ```
//!
//! Without the env vars each test prints why it is skipping and passes.

use std::path::PathBuf;

use embed_onnx::OnnxEmbedder;
use transcribe_core::SentenceEmbedder;

fn embedder() -> Option<OnnxEmbedder> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    match (var("PARAGRAPH_TEST_MODEL"), var("PARAGRAPH_TEST_TOKENIZER")) {
        (Some(model), Some(tokenizer)) => Some(OnnxEmbedder::new(model, tokenizer).unwrap()),
        _ => {
            eprintln!("SKIPPED: set PARAGRAPH_TEST_MODEL and PARAGRAPH_TEST_TOKENIZER (see the header of tests/real_model.rs)");
            None
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}

#[test]
#[ignore = "slow: loads the paragraph model (PARAGRAPH_TEST_MODEL, PARAGRAPH_TEST_TOKENIZER)"]
fn related_sentences_are_closer_than_unrelated_ones_across_languages() {
    let Some(embedder) = embedder() else {
        return;
    };
    let v = embedder
        .embed(&[
            "The cat is sleeping on the sofa.",
            "Le chat dort sur le canapé.",
            "My tax return is due next week.",
        ])
        .unwrap();
    assert_eq!(v.len(), 3);
    assert!(v.iter().all(|e| e.len() == 384), "dimension {}", v[0].len());
    assert!(
        cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]) + 0.2,
        "{} vs {}",
        cosine(&v[0], &v[1]),
        cosine(&v[0], &v[2])
    );
}

#[test]
#[ignore = "slow: loads the paragraph model (PARAGRAPH_TEST_MODEL, PARAGRAPH_TEST_TOKENIZER)"]
fn batching_and_padding_do_not_change_embeddings() {
    let Some(embedder) = embedder() else {
        return;
    };
    let long = "This sentence is much longer than the other one, so the short one gets padded.";
    let alone = embedder.embed(&["Short."]).unwrap();
    let padded = embedder.embed(&[long, "Short."]).unwrap();
    // Padding shifts the int8 model's output slightly (about 0.997 in
    // practice), far below what changes a drift decision.
    assert!(
        cosine(&alone[0], &padded[1]) > 0.99,
        "{}",
        cosine(&alone[0], &padded[1])
    );

    // More sentences than one batch, and an input longer than the token limit.
    let many: Vec<String> = (0..70).map(|i| format!("Sentence number {i}.")).collect();
    let mut refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let huge = "word ".repeat(500);
    refs.push(&huge);
    assert_eq!(embedder.embed(&refs).unwrap().len(), 71);
}
