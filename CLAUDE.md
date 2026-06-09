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

## Status

**M1 complete (both platforms wired end-to-end for romaji→kana):**
- Core: real incremental romaji→kana in `ime-engine` (`romaji.rs`); 33 workspace tests pass.
- macOS: `RomajiIME.app` builds (universal), links the engine via the C ABI, launches its
  IMKServer, installs to `~/Library/Input Methods/`. Build: `platform/macos/build.sh`.
  (Enabling it + typing in TextEdit is a manual GUI step.)
- Windows: `ime-server` (Rust) hosts the engine over a named pipe — verified to cross-compile
  for x86_64/i686-pc-windows-msvc and host-tested via the dispatcher/transport unit tests.
  The thin C++ TSF DLL (`platform/windows`) is written but builds on Windows/CI (not on macOS).

Next: **M2 — cloud-AI conversion (the headline feature)** + candidate UI on both frontends.
