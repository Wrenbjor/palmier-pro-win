//! Throwaway probe for infra/ffmpeg-toolchain.
//!
//! If this compiles, the ffmpeg-sys-next build script found the headers (FFMPEG_DIR)
//! and libclang (LIBCLANG_PATH) and linked the import .libs. If it RUNS and prints a
//! version, the avcodec-61.dll / avutil-59.dll / ... were resolvable at load time
//! (C:\ffmpeg\bin on PATH). That end-to-end is exactly the toolchain stories E5-S2,
//! E4-S3/S4/S5, E6-S5 need.

fn main() {
    ffmpeg_next::init().expect("ffmpeg_next::init() failed");

    // avutil version (the lib all others depend on), decoded into MAJOR.MINOR.MICRO.
    let v = ffmpeg_next::util::version();
    println!(
        "ffmpeg-next OK: libavutil version {} ({}.{}.{})",
        v,
        v >> 16,
        (v >> 8) & 0xff,
        v & 0xff
    );

    // Human-readable configuration string straight from the linked FFmpeg.
    println!("ffmpeg configuration: {}", ffmpeg_next::util::configuration());
    println!("PROBE_SUCCESS");
}
