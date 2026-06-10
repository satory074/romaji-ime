//! Smoke-test + latency/streaming check for the real cloud-AI converter.
//! Usage: cargo run --example ai_smoke -- <data_dir_with_config.json>
//! Hits the live API via the engine's actual HTTP path (never prints the key).

use ime_engine::ai::{converter_from_config, ConvertRequest};
use std::path::PathBuf;
use std::time::Instant;

fn req(romaji: &str) -> ConvertRequest {
    ConvertRequest {
        romaji: romaji.to_string(),
        kana: String::new(),
        context_before: String::new(),
        context_after: String::new(),
    }
}

fn main() {
    let dir = std::env::args().nth(1).map(PathBuf::from);
    let Some(conv) = converter_from_config(dir.as_deref()) else {
        println!("no converter configured (check config.json / env)");
        return;
    };

    // Streaming: each sink callback is a growing candidate list. The first
    // callback (first candidate) should land well before the final one.
    println!("=== streaming: kyouhaiitenkidesune ===");
    let t = Instant::now();
    let mut sink = |cands: Vec<String>| {
        println!("  +{:>5} ms  {cands:?}", t.elapsed().as_millis());
    };
    if let Err(e) = conv.convert_streaming(&req("kyouhaiitenkidesune"), &mut sink) {
        println!("  ERROR: {e}");
    }

    // Same input again -> cache hit (instant, no network).
    println!("=== cache hit (same input) ===");
    let t2 = Instant::now();
    match conv.convert(&req("kyouhaiitenkidesune")) {
        Ok(c) => println!("  {:>5} ms  {c:?}", t2.elapsed().as_millis()),
        Err(e) => println!("  ERROR: {e}"),
    }
}
