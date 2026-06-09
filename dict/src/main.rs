//! Dictionary build tool (stub).
//!
//! In M3 this compiles SudachiDict (Apache-2.0) source CSVs from `dict/source/`
//! into compact, mmap-friendly binary artifacts (a reading-keyed trie, surface +
//! cost tables, and the connection-cost matrix) consumed by `ime-engine`'s
//! `LocalConverter`. Deliberately uses only license-clean (Apache-2.0 / BSD)
//! dictionary data so the IME can ship closed-source/commercial.

fn main() {
    eprintln!(
        "ime-dict (stub): compiles SudachiDict -> binary trie/cost tables. \
         Implemented in milestone M3."
    );
}
