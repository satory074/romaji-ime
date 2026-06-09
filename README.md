# romaji-ime

A cross-platform (Windows + macOS) Japanese romaji IME with a **cloud-LLM "loose
romaji → AI conversion"** headline feature, backed by a fast offline local
converter.

- **Windows** frontend: a thin TSF (Text Services Framework) TIP DLL that talks
  over a named pipe to a separate `ime-server` process hosting the engine.
- **macOS** frontend: an InputMethodKit (`.app`) bundle that links the engine
  in-process via a C ABI.
- **Shared core** (`crates/ime-engine`): OS-independent. romaji→kana, a session
  state machine, and two converters behind one trait — a `LocalConverter`
  (dictionary + Viterbi, offline/fallback) and a `CloudAiConverter` (LLM API,
  the headline feature; async, with timeout and local fallback).

See the implementation plan for the full design and milestones (M0–M5+).

## Repository layout

| Path | What |
|------|------|
| `crates/ime-engine` | Core engine. Pure Rust, no FFI, unit-tested. |
| `crates/ime-ffi`    | C ABI shim (`cdylib`+`staticlib`). macOS contract. Generates `romaji_ime.h`. |
| `crates/ime-ipc`    | IPC message types + framing. Windows contract. |
| `crates/ime-server` | Named-pipe server hosting the engine (Windows). |
| `dict`              | Build tool: compiles SudachiDict → binary trie / cost tables. |
| `xtask`             | Dev automation (header gen, dict build, mac packaging). |
| `platform/windows`  | C++ TSF TIP DLL + installer. |
| `platform/macos`    | Swift IMK app + custom candidate window. |

## Build (host = macOS)

```bash
cargo build --workspace          # core, ffi, ipc, server (host arch)
cargo test  -p ime-engine        # core unit tests
cargo run   -p xtask -- gen-header   # regenerate crates/ime-ffi/include/romaji_ime.h
```

Windows artifacts are built/verified on a Windows machine or CI (see `ci/`).
