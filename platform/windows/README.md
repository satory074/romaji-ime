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

## Scope (M1)

- ✅ Romaji→kana via the shared engine, shown as a TSF composition; Enter/Space commit.
- ✅ `ITfTextInputProcessorEx` + UI-less / immersive category registration (works in UWP/Store apps).
- ⛔ Candidate-window UI (`ITfCandidateListUIElement`) — M2 (cloud-AI conversion).
- ⛔ Auto-launch / restart of `ime-server.exe`, randomized pipe name, multi-client concurrency — M4/M5.
- ⛔ Authenticode signing + installer — M5.
