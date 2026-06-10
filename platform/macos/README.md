# RomajiIME — macOS frontend (InputMethodKit)

A Swift `.app` input method that links the Rust engine **in-process** via the C
ABI (`crates/ime-ffi` → `romaji_ime.h`) and reflects preedit/commit through IMK.
macOS already isolates the IME in its own process, so (unlike Windows) there is
no separate server.

## Build & install

```bash
./build.sh            # universal RomajiIME.app under build/
./build.sh --install  # also copy to ~/Library/Input Methods/
ARCHS="arm64" ./build.sh   # host-arch only (faster local builds)
```

The script builds the engine as a universal cdylib, compiles the Swift sources
with `swiftc` (Swift 5 language mode to avoid Swift 6's `IMKInputController`
concurrency constraint — IMKSwift is the alternative for full Swift 6), assembles
and ad-hoc signs the bundle.

## Enable it (one-time, GUI)

System Settings ▸ Keyboard ▸ Text Input ▸ Edit… ▸ **+** ▸ Japanese ▸ **RomajiIME**,
then switch to it (Ctrl+Space / the input menu) and type e.g. `konnichiha` in
TextEdit → **こんにちは**. Enter commits the kana.

## Cloud-AI conversion (the headline feature) — no mode switching

You just type loose romaji; the AI does the rest. **There is no kana/English
mode to toggle.**

- While composing, the **raw romaji is shown** (so English/identifiers look
  natural). Type Japanese romaji and English freely in one go.
- **Auto-convert:** ~0.5 s after you stop typing, the AI converts the whole
  composition. You can also press **Space** or **Enter** to convert immediately.
- The AI keeps intended English/Latin (e.g. `github`→`GitHub`, `ok`→`OK`) while
  converting the Japanese (e.g. `nihongo wo github de kanri` → `日本語をGitHubで管理`).
- In the candidate list: **Space/↓** cycle, **↑** back, **number keys / Enter**
  commit, **Esc** cancels back to the romaji.

Configure by creating **`~/Library/Application Support/RomajiIME/config.json`**
(see `docs/config.example.json`) with your provider + API key, then re-select the
input source so the engine reloads. Keystrokes are never sent to the cloud in
secure (password) fields, and the API key is never logged or committed. Without
AI configured, it falls back to plain romaji→kana (kana shown inline, Enter
commits).

Candidates appear in a **custom list window** below the caret
(`CandidateWindow.swift`, a non-activating `NSPanel` — not `IMKCandidates`); the
highlighted candidate is also shown inline as the marked text. Auto-convert calls
the API on each typing pause — see the cost note in the main README /
`docs/config.example.json`.

## Verifying without a GUI

You can confirm the bundle links the engine and starts its IMKServer:

```bash
./build/RomajiIME.app/Contents/MacOS/RomajiIME   # logs: "IMKServer started … engine ABI=1"
```

Debugging tip: never attach a debugger to a *live* (system-activated) input
method — it freezes text input. Use `NSLog` + Console.app, and unit-test logic in
`crates/ime-engine`.

## Files

| File | Role |
|------|------|
| `Sources/RomajiIME/main.swift`           | Process entry: creates `IMKServer`. |
| `Sources/RomajiIME/InputController.swift`| `IMKInputController`: key → engine → marked text/commit. |
| `Sources/RomajiIME/EngineBridge.swift`   | Swift wrapper over the C ABI. |
| `Sources/RomajiIME/Bridging-Header.h`    | Imports `romaji_ime.h`. |
| `Resources/Info.plist`                   | IMK wiring (connection name, controller class, input mode). |
| `RomajiIME.entitlements`                 | Sandbox + mach-register + network (for M2/M5 signed builds). |

## Scope (M1)

- ✅ Romaji→kana shown inline (marked text); Enter/Space commit.
- ⛔ Custom `NSWindow` candidate window — M2 (cloud-AI conversion).
- ⛔ Developer ID signing + notarization + sandbox — M5.
