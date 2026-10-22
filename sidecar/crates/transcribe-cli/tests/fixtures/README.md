# Test fixtures

## `jfk-ask-not.wav`

- **Content:** the first 4.8 s of John F. Kennedy's inaugural address
  (20 January 1961): *"And so, my fellow Americans, ask not"*.
- **Format:** RIFF WAV, 16-bit signed PCM, mono, 16 kHz, 76 800 samples.
- **Source:** `samples/jfk.wav` from the whisper.cpp repository
  (<https://github.com/ggml-org/whisper.cpp/blob/master/samples/jfk.wav>,
  SHA-256 `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e`),
  trimmed with `sox jfk.wav jfk-ask-not.wav trim 0 4.8 fade t 0 4.8 0.3`.
- **License:** public domain. The recording is a work of the United States
  federal government (a presidential address), not subject to copyright
  (17 U.S.C. § 105).
- **Expected words** (case-insensitive, checked by `tests/real_model.rs`):
  `fellow`, `americans`. A correct transcript reads roughly
  "And so, my fellow Americans, ask not".
- **Timing:** speech 0.3–2.0 s ("And so my fellow Americans"), pause
  2.0–3.3 s, "ask" 3.3–3.6 s, "not" 4.0–4.2 s, silence to the end.
