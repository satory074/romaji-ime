# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A cross-platform (Windows + macOS) Japanese **romaji IME** whose headline feature
is cloud-LLM **"loose romaji → AI conversion"** (type imprecise romaji, an LLM
produces correct Japanese — Sumibi-style), with a fast offline romaji→kana
fallback.

Product decisions that shape the architecture (don't silently reverse them):
- **No mode switching.** This is the whole point: the user never toggles
  kana/English. While composing, the preedit shows the **raw romaji** (so English
  looks natural), and the AI converts the whole buffer — keeping intended
  English/Latin (`github`→`GitHub`) while converting the Japanese.
- **Cloud LLM is the headline converter**, not on-device. Runs async with a
  timeout; falls back to the local romaji→kana on error/unavailable.
- **Never block the host app's input thread.** The slow LLM call runs off the
  key-event path (a separate `ime-server` process on Windows; a background
  DispatchQueue + main-thread polling on macOS).
- **Closed-source / commercial distribution is in view** → permissive deps/data
  only (wana_kana MIT, vibrato Apache/MIT, SudachiDict Apache-2.0). Do **NOT**
  bundle GPL MeCab dictionaries or **CC-BY-SA Zenzai weights**; the cloud-API
  approach ships no model weights.

Full design + milestones (M0–M5+) live in `~/.claude/plans/windows-shimmying-bunny.md`.

## Architecture

A Rust **shared core** plus **thin platform frontends**. The core exposes the
*same* engine through two contracts:

- **macOS** links the core **in-process** via a **C ABI** (`crates/ime-ffi` →
  generated `crates/ime-ffi/include/romaji_ime.h`). IMK already isolates the IME
  in its own process, so no separate server is needed.
- **Windows** runs the core in a **separate `ime-server` process**; the thin TSF
  DLL talks to it over a **named pipe** using the IPC types in `crates/ime-ipc`
  (length-prefixed bincode). Gives crash isolation + a non-blocking key sink.

```
crates/ime-engine  Core (pure Rust, no FFI, unit-tested): romaji.rs (romaji→kana),
                   Session state machine (Composing/Candidates modes), ai.rs
                   (Converter trait + HttpConverter, async begin/poll).
crates/ime-ffi     C ABI shim (staticlib+cdylib). macOS contract. cbindgen header.
crates/ime-ipc     IPC message types + length-prefixed bincode framing. Windows contract.
crates/ime-server  Named-pipe server hosting the engine (Windows): dispatch.rs +
                   transport.rs (host-testable) + pipe_win.rs (#[cfg(windows)]).
dict/              Build tool: SudachiDict → binary trie/cost tables (M3, stub).
xtask/             Dev automation (gen-header today).
platform/macos/    Swift IMK .app (built by build.sh, no Xcode project).
platform/windows/  C++ TSF TIP DLL + ipc client (built on Windows/CI via CMake).
docs/              c-abi / ipc-protocol specs, config.example.json.
```

**One workspace, two outputs** by crate split (not `cfg`): macOS builds `ime-ffi`
for both apple targets and `lipo`s them in; Windows builds `ime-server` (which
links `ime-engine`) — the thin C++ DLL links neither, its contract is the IPC
protocol. A 64-bit server can serve both 32- and 64-bit DLL clients, but **both
32- and 64-bit DLLs must ship** (a TSF DLL loads into each host app's bitness).

### Request flow (the conversion path)
1. Frontend translates the native key event → platform-neutral `ime_engine::Key`
   (X11/IBus-style keysym). The engine never sees OS key codes.
2. Printable keys append to the session's raw romaji buffer; the frontend shows
   it as the preedit and (macOS) schedules a debounced auto-convert.
3. On a pause / Space / Enter, the frontend calls `begin_ai_convert` (returns a
   request id; the LLM runs on a background thread) and **polls** on the session
   thread until ready. On ready the session enters Candidates mode.
4. Candidates: Space/↓ cycle, ↑ back, number keys / Enter commit, Esc cancels.
   On error/unavailable the frontend falls back to committing local kana.

## Commands

```bash
cargo build --workspace                  # core + ffi + ipc + server (host arch)
cargo test  --workspace                  # all unit tests
cargo test  -p ime-engine romaji         # single crate, tests matching "romaji"
cargo test  -p ime-engine session::tests::enter_commits_kana   # one exact test
cargo fmt --all                          # format (CI expects --check clean)
cargo clippy --workspace
cargo run -p xtask -- gen-header         # regenerate romaji_ime.h after C-ABI changes

# Live cloud-AI smoke test through the engine's real HTTP path (needs config.json):
cargo run -p ime-engine --example ai_smoke -- "$HOME/Library/Application Support/RomajiIME"

# macOS app (no Xcode project; swiftc + bundle assembly in build.sh):
platform/macos/build.sh                  # universal RomajiIME.app under build/
platform/macos/build.sh --install        # also copy to ~/Library/Input Methods/
ARCHS="arm64" platform/macos/build.sh     # host-arch only (faster iteration)

# Windows: can't build the C++ TSF DLL on macOS — only cross-typecheck the Rust:
cargo check -p ime-server --target x86_64-pc-windows-msvc
cargo check -p ime-server --target i686-pc-windows-msvc
# On Windows: cmake the DLL (platform/windows/README.md) + cargo build -p ime-server,
# then regsvr32 the per-arch DLLs.
```

After reinstalling the macOS app, **re-select the input source** (it's killed so
the system relaunches a fresh process that reloads `config.json`). To register/
enable it without a logout, `TISRegisterInputSource` on the bundle (see how the
session does it). Don't attach a debugger to a *live* IME — it freezes input;
use the file log (below) + Console.app.

## Conventions / gotchas

- **Result flags are ABI**: CONSUMED=1, PREEDIT=2, CANDIDATES=4, COMMIT=8
  (`ime_engine::flags`, mirrored as `RIME_*` in the header). Renumbering = ABI
  break → bump `rime_abi_version`.
- **IPC byte layout is pinned** by `ime_ipc::tests::process_key_byte_layout_is_stable`.
  If it changes, update `docs/ipc-protocol.md` **and** the hand-written C++ codec
  (`platform/windows/src/ipc.cpp`) together.
- **C-ABI `const char*` are session-owned**, valid only until the next mutating
  call on that session; callers copy out immediately.
- **`begin_ai_convert`/`poll_ai_result` must be called on the same thread** as
  other session calls. Only the HTTP request runs on another thread (it writes a
  shared slot); the `Session` is never touched concurrently. A frontend keystroke
  cancels any in-flight/pending conversion.
- **macOS Shift+symbol** uses `event.characters` (the actual produced char, so
  Shift+1 → "!"); only ASCII letters are lowercased to keep romaji
  case-insensitive. Ctrl/Alt combos are shortcuts, not text.
- **ureq with only the `native-tls` feature** needs an explicit
  `.tls_connector(...)` or HTTPS fails instantly. native-tls (SChannel on
  Windows) is chosen so it cross-compiles without ring/OpenSSL.
- **Swift 6** can't directly subclass `IMKInputController`; this app builds with
  `-swift-version 5` (IMKSwift is the alternative). `InputController` is
  `@objc(InputController)` to match `InputMethodServerControllerClass`.
- **Auto-convert calls the API on each typing pause** (~500ms debounce) — mind
  rate limits / cost; on Gemini free tier a 429 falls back to local kana.
- Rust toolchain pinned in `rust-toolchain.toml` (stable + the 4 targets).
- Diagnostics: the macOS app writes `~/Library/Application Support/RomajiIME/debug.log`
  (key never logged); `rime_get_last_error` surfaces the last AI error.

## Cloud-AI configuration

`ime-engine/ai.rs`: `Converter` trait + `HttpConverter` for OpenAI-compatible,
Anthropic, and Gemini (provider `"gemini"` auto-targets Google's OpenAI-compatible
endpoint), behind the default `cloud-http` feature. Config is read from
`{user_data_dir}/config.json` (macOS: `~/Library/Application Support/RomajiIME/`)
or `ROMAJI_IME_*` env vars; see `docs/config.example.json`. `Engine::new` is pure
— frontends opt in via `with_ai_from_config()`.

## Status

- **Core + macOS (M0–M2): verified working on-device.** Type loose romaji → AI
  converts (incl. mixed English, e.g. `nihongo wo github de kanri` → `日本語を
  GitHubで管理`); auto-convert on pause; Space/Enter convert immediately;
  candidate-list window below the caret (`CandidateWindow.swift`, custom NSPanel —
  not IMKCandidates) with Space/↓ cycle, number/Enter commit, Esc cancel. Builds
  via `platform/macos/build.sh`.
- **Windows:** `ime-server` (Rust) hosts the engine + AI over a named pipe;
  cross-compiles for x86_64/i686-pc-windows-msvc; dispatcher/transport
  unit-tested. The C++ TSF DLL does romaji→kana (M1) and has the IPC client incl.
  AI calls; its **async AI trigger + candidate UI must still be built/iterated on
  Windows** (design in `platform/windows/README.md`).

Pending: Windows AI trigger/candidate UI; M3 (local dictionary/Viterbi fallback);
M4 (learning); M5 (signing/installers/notarization).
