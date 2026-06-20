// THROWAWAY whisper-rs build/smoke probe.
//
// Primary goal: prove `whisper-rs` (and thus whisper.cpp, built from source via
// cmake + bindgen through the MSVC wrapper) COMPILES AND LINKS on this box.
// Just building this crate is the real signal.
//
// Secondary (optional) smoke: if a GGML model path is given as argv[1] and a 16 kHz
// mono f32 WAV path as argv[2], it actually runs a transcription so we confirm the
// runtime works end to end. With no args it prints the linked whisper.cpp version and
// exits 0 — the build alone is the verification.
//
// Run (build):   pwsh -File ../../scripts/with-msvc.ps1 cargo build
// Run (smoke):   pwsh -File ../../scripts/with-msvc.ps1 cargo run -- <model.bin> <audio16k.wav>

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() {
    // Prove the FFI is linked: this calls into whisper.cpp.
    println!("whisper.cpp system info: {}", whisper_rs::print_system_info());

    let mut args = std::env::args().skip(1);
    let model_path = match args.next() {
        Some(p) => p,
        None => {
            println!("BUILD OK: whisper-rs linked. No model arg given — skipping transcription smoke.");
            println!("To smoke-test: cargo run -- <ggml-model.bin> <audio_16k_mono.wav>");
            return;
        }
    };
    let audio_path = args.next().expect("usage: whisper-probe <model.bin> <audio16k.wav>");

    let ctx = WhisperContext::new_with_params(&model_path, WhisperContextParameters::default())
        .expect("failed to load model");
    let mut state = ctx.create_state().expect("failed to create state");

    // Read a 16-bit PCM WAV and convert to the f32 mono whisper.cpp expects.
    let samples = read_wav_i16_to_f32(&audio_path);

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    state.full(params, &samples).expect("transcription failed");

    // whisper-rs 0.16: full_n_segments() -> i32; get_segment(i) -> Option<WhisperSegment>.
    let n = state.full_n_segments();
    println!("--- transcription ({n} segments) ---");
    for i in 0..n {
        if let Some(seg) = state.get_segment(i) {
            // Timestamps are centiseconds (1/100 s) -> seconds for the real pipeline mapping.
            let start = seg.start_timestamp() as f64 / 100.0;
            let end = seg.end_timestamp() as f64 / 100.0;
            let text = seg.to_str_lossy().unwrap_or_default();
            println!("[{start:.2} -> {end:.2}] {text}");
        }
    }
}

fn read_wav_i16_to_f32(path: &str) -> Vec<f32> {
    // Minimal WAV reader: assumes 16-bit PCM, mono, 16 kHz (what FFmpeg will produce
    // for the real pipeline). Good enough for a throwaway smoke test.
    let bytes = std::fs::read(path).expect("read wav");
    let data_start = find_data_chunk(&bytes).expect("no data chunk");
    bytes[data_start..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

fn find_data_chunk(bytes: &[u8]) -> Option<usize> {
    let mut i = 12; // skip RIFF header
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        if id == b"data" {
            return Some(i + 8);
        }
        i += 8 + size;
    }
    None
}
