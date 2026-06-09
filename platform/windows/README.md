# RomajiIME — Windows frontend (TSF)

A thin **TSF Text Input Processor (TIP)** DLL that runs inside every app, plus
the separate **`ime-server.exe`** (Rust, `crates/ime-server`) that hosts the
engine. The DLL forwards key events to the server over a named pipe
(`\\.\pipe\romaji_ime`) and reflects the returned preedit/commit into the
document via TSF compositions. This keeps the in-every-process DLL tiny and
crash-proof and keeps all heavy work off the host app's input thread.

> **Build platform:** Windows + Visual Studio 2022 + Windows SDK. These sources
> cannot be compiled on the macOS dev host. The Rust server and the IPC wire
> format *are* verified on macOS (`cargo check --target x86_64-pc-windows-msvc`
> and `ime-ipc`'s byte-layout test); the C++ here is built/iterated on Windows
> or CI.

## Layout

| File | Role |
|------|------|
| `src/ipc.{h,cpp}`        | Named-pipe client + bincode codec (mirrors `docs/ipc-protocol.md`). |
| `src/TextService.{h,cpp}`| The TIP: `ITfTextInputProcessorEx` + key sink + composition. |
| `src/dllmain.cpp`        | DLL exports, class factory, COM + TSF registration. |
| `src/Globals.{h,cpp}`    | CLSID / profile GUID / constants. |
| `src/RomajiIME.def`      | Exported DLL entry points. |
| `CMakeLists.txt`         | Build (x64 and Win32). |

## Build

```bat
:: 1) Build the engine server (host arch; ship matching/64-bit):
cargo build --release -p ime-server

:: 2) Build the TIP DLL for both architectures (TSF requires both):
cmake -B build-x64 -A x64   && cmake --build build-x64 --config Release
cmake -B build-x86 -A Win32 && cmake --build build-x86 --config Release
```

## Install / register (elevated)

```bat
regsvr32 build-x64\Release\RomajiIME.dll
regsvr32 build-x86\Release\RomajiIME.dll
:: start the engine server (Run-at-login wiring comes in M5)
start "" ime-server.exe
```

Then add **RomajiIME** under Settings ▸ Time & language ▸ Language ▸ Japanese ▸
Keyboards, switch to it, and type e.g. `konnichiha` → こんにちは. Unregister with
`regsvr32 /u`.

## Cloud-AI conversion on Windows (design — remaining work)

The engine and IPC fully support AI: `ime-server` handles `BeginAiConvert` /
`PollAiResult`, and `PipeClient` (here) implements both calls. What remains is
wiring them into the TIP **without blocking the input thread** — the LLM round
trip takes ~1–2 s and the DLL runs inside every app, so polling synchronously in
`OnKeyDown` would freeze the host (the top IME anti-pattern).

Planned flow (to build/iterate on Windows):
1. On Space while composing, `OnKeyDown` calls `PipeClient::BeginAiConvert` and
   returns immediately (key eaten); show the kana meanwhile.
2. A message-only window polls `PollAiResult` on a `WM_TIMER` (off the keystroke
   path). On `Ready`, run an edit session to display the highlighted candidate;
   Space/↓ cycle, number/Enter commit, Esc cancels — mirroring the engine's
   candidate mode (and the macOS frontend).
3. Optionally surface the list via `ITfCandidateListUIElement` (UI-less mode);
   an inline highlighted-candidate display (like macOS) is the simpler first cut.

`begin`/`poll` must run off the UI thread (a worker thread or the timer window),
never inside `OnKeyDown`.

## Scope

- ✅ Romaji→kana via the shared engine, shown as a TSF composition; Enter/Space commit.
- ✅ `ITfTextInputProcessorEx` + UI-less / immersive category registration (works in UWP/Store apps).
- ✅ IPC client for AI (`BeginAiConvert`/`PollAiResult`) matching `docs/ipc-protocol.md`.
- ⛔ Async AI trigger + candidate rendering in the TIP (design above) — build on Windows.
- ⛔ Auto-launch / restart of `ime-server.exe`, randomized pipe name, multi-client concurrency — M4/M5.
- ⛔ Authenticode signing + installer — M5.
