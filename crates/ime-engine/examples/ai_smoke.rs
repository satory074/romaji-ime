//! Smoke-test the real cloud-AI converter against the configured provider.
//! Usage: cargo run --example ai_smoke -- <data_dir_with_config.json>
//! Hits the live API using the engine's actual HTTP path (does NOT print the key).

use ime_engine::ai::{converter_from_config, ConvertRequest};
use std::path::PathBuf;

fn main() {
    let dir = std::env::args().nth(1).map(PathBuf::from);
    let Some(conv) = converter_from_config(dir.as_deref()) else {
        println!("no converter configured (check config.json / env)");
        return;
    };
    let req = ConvertRequest {
        romaji: "nihongo".to_string(),
        kana: "にほんご".to_string(),
        context_before: String::new(),
        context_after: String::new(),
    };
    match conv.convert(&req) {
        Ok(candidates) => println!("OK candidates: {candidates:?}"),
        Err(e) => println!("ERROR: {e}"),
    }
}
