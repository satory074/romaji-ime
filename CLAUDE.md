# CLAUDE.md — romaji-ime

Guidance for Claude Code when working in this repository.

## What this is

A cross-platform (Windows + macOS) Japanese **romaji IME** whose **headline
feature is cloud-LLM "loose romaji → AI conversion"** (type imprecise romaji,
let an LLM produce correct Japanese — Sumibi-style), backed by a fast **offline
local converter** as fallback.

Key product decisions (these shape the architecture; don't silently reverse them):
- **Cloud LLM is the headline converter**, not on-device. Triggered on the
  conversion key, run async, with timeout + fallback to the local converter.
- **Closed-source / commercial distribution is in view.** Therefore: only
  permissive deps/data (wana_kana MIT, vibrato Apache/MIT, SudachiDict
  Apache-2.0). **Do NOT bundle** GPL MeCab dictionaries or **CC-BY-SA Zenzai
  model weights**. The cloud-API approach means no model weights are shipped.
- **Never block the input thread.** AI/dictionary work happens off the host
  app's key-event path (separate process on Windows; background queue on macOS).

The full design + milestones (M0–M5+) live in the approved plan at
`~/.claude/plans/windows-shimmying-bunny.md`.

## Architecture

Rust shared core + thin platform frontends. The core exposes two contracts over
the *same* engine:
- **macOS** links the core in-process via a **C ABI** (`crates/ime-ffi` →
  `romaji_ime.h`). IMK already isolates the IME in its own process.
- **Windows** runs the core in a **separate `ime-server` process**; the thin TSF
  DLL talks to it over a **named pipe** using the **IPC types in `crates/ime-ipc`**
  (length-prefixed bincode). This gives crash isolation + a non-blocking key sink.

```
crates/ime-engine  Core. Pure Rust, no FFI, unit-tested. romaji→kana, Session
                   state machine, and (later) LocalConverter + CloudAiConverter
                   behind one trait.
crates/ime-ffi     C ABI shim (staticlib+cdylib). macOS contract. cbindgen header.
crates/ime-ipc     IPC message types + framing. Windows contract.
crates/ime-server  Named-pipe server hosting the engine (Windows).
dict/              Build tool: SudachiDict → binary trie/cost tables (M3).
xtask/             Dev automation (header gen, dict build, mac packaging).
platform/windows/  C++ TSF TIP DLL + installer (built on Windows/CI).
platform/macos/    Swift IMK .app + custom NSWindow candidate window.
```

**One workspace, two outputs** by crate split (not `cfg`): macOS builds
`ime-ffi` for both apple targets and `lipo`s them; Windows builds `ime-server`
(links `ime-engine`); the thin DLL links neither — its contract is the IPC
protocol. A 64-bit server can serve both 32- and 64-bit DLL clients, but **both
32- and 64-bit DLLs must ship** (TSF DLLs load into each host app's bitness).

## Build / test

```bash
cargo build --workspace                 # core, ffi, ipc, server (host arch)
cargo test  --workspace                 # unit tests (engine, ffi, ipc)
cargo run   -p xtask -- gen-header       # regenerate crates/ime-ffi/include/romaji_ime.h

# macOS universal staticlib inputs:
cargo build -p ime-ffi --target aarch64-apple-darwin
cargo build -p ime-ffi --target x86_64-apple-darwin
# then: lipo -create ... -output libime_ffi.a

# Windows is built/verified on a Windows machine or CI. From macOS you can only
# type-check (no MSVC linker locally):
cargo check -p ime-server --target x86_64-pc-windows-msvc
cargo check -p ime-server --target i686-pc-windows-msvc
```

## Gotchas / conventions

- **Key events are platform-neutral.** Frontends translate native VK / NSEvent
  codes into `ime_engine::Key` (X11/IBus-style keysyms) before calling the
  engine. The engine never sees OS key codes.
- **Result flags are part of the ABI**: CONSUMED=1, PREEDIT=2, CANDIDATES=4,
  COMMIT=8 (`ime_engine::flags`). Renumbering = ABI break (bump
  `rime_abi_version`).
- **IPC byte layout is pinned** by a test in `ime-ipc`
  (`process_key_byte_layout_is_stable`). If it fails, the wire format changed —
  update `docs/ipc-protocol.md` and the hand-written C++ codec deliberately.
- **Returned `const char*` from the C ABI are session-owned**, valid only until
  the next mutating call on that session. Callers copy out immediately.
- **macOS Swift 6** cannot directly subclass `IMKInputController`; use IMKSwift
  or target-wide `@MainActor`. Don't attach a debugger to a live IME (freezes
  input) — verify via tests + `NSLog`/Console.app.
- Rust toolchain pinned in `rust-toolchain.toml` (stable + 4 targets). LLM API
  keys must never be committed (see `.gitignore`).

## Cloud-AI conversion (headline feature)

Type loose romaji, press **Space**, an LLM returns ranked Japanese candidates
(↓/Space cycle, number/Enter commit, Esc cancel). Engine: `ime-engine/ai.rs`
(`Converter` trait + `HttpConverter` for OpenAI-compatible/Anthropic, feature
`cloud-http`). The LLM call runs on a background thread; the session is polled on
its own thread (begin/poll pull model) so the input thread never blocks.
Configure via `{user_data_dir}/config.json` or `ROMAJI_IME_*` env (see
`docs/config.example.json`). `native-tls` keeps it cross-compiling to Windows.
Never sends keystrokes from secure (password) fields.

## Status

- **Core (M1+M2):** romaji→kana + cloud-AI candidate machine in `ime-engine`; 41 workspace tests.
- **macOS (M1+M2): ✅ verified working end-to-end on-device.** `RomajiIME.app` (universal) —
  romaji→kana inline, **Space = AI convert** with inline candidate cycling, secure-field guard.
  Live Gemini conversion confirmed (konnichiha→こんにちは, 私は日本語を話します, etc.). Builds via
  `platform/macos/build.sh`, installs to `~/Library/Input Methods/`. A custom candidate-list
  window is still pending (candidates show inline, cycled with Space).
  Gotcha fixed: ureq w/ only native-tls needs an explicit `.tls_connector(...)` or HTTPS fails.
  Diagnostics: file log at `~/Library/Application Support/RomajiIME/debug.log`;
  `cargo run -p ime-engine --example ai_smoke -- <data_dir>` tests the live API path.
- **Windows:** `ime-server` (Rust) hosts the engine + AI over a named pipe; cross-compiles for
  x86_64/i686-pc-windows-msvc; dispatcher/transport unit-tested. The C++ TSF DLL does romaji→kana
  (M1); its **candidate UI + Space-triggered AI are still to do (M2d)** and it builds on Windows/CI.

Next: Windows candidate UI (M2d); then M3 (local dictionary/Viterbi fallback), M4 (learning), M5 (signing/installers).
