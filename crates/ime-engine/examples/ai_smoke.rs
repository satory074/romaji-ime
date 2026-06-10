//! Smoke-test + latency check for the real cloud-AI converter.
//! Usage: cargo run --example ai_smoke -- <data_dir_with_config.json>
//! Hits the live API via the engine's actual HTTP path (never prints the key).
//! Times three calls to show the effect of connection reuse and the cache.

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
    // 1) cold: new TLS connection. 2) different input: warm (reused) connection,
    // no cache. 3) repeat of #1: cache hit (no network).
    for (label, romaji) in [
        ("cold     ", "nihongo"),
        ("warm-conn", "kyouhaiitenki"),
        ("cache-hit", "nihongo"),
    ] {
        let t = Instant::now();
        match conv.convert(&req(romaji)) {
            Ok(c) => println!(
                "[{label}] {:>5} ms  {romaji:?} -> {c:?}",
                t.elapsed().as_millis()
            ),
            Err(e) => println!("[{label}] ERROR: {e}"),
        }
    }
}
