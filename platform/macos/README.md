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
- **"Space converts, Enter commits what you see."** ~0.5 s after you stop typing,
  the AI converts the whole composition and shows it as a **preview** — the
  candidates appear below, but the inline text stays the raw romaji, so **Enter
  commits exactly what you typed** (handy for English / things you don't want
  converted). Press **Space** to *engage* the conversion (the inline text becomes
  the top candidate); **Enter** then commits that candidate, and more **Space**
  cycles. So Enter only converts once **you** asked it to with Space — a pause
  never silently changes what Enter does.
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

## Send it to a (Mac) friend — unsigned zip, bring-your-own-free-key

For a handful of (semi-technical) friends, you don't need an Apple Developer
account. `package-zip.sh` bundles the built app with an end-user `install.sh`
and `INSTALL.md`:

```bash
platform/macos/package-zip.sh        # -> build/RomajiIME-macos.zip
```

Send that zip. The friend unzips and runs `./install.sh`, which clears the
Gatekeeper quarantine (ad-hoc build, not notarized), installs to
`~/Library/Input Methods/`, and writes `config.json` from **their own free
Gemini key** (https://aistudio.google.com/apikey) — no key of yours is ever
shipped. `INSTALL.md` is the friend-facing walkthrough. The cloud-AI feature
needs a key, so each user supplies their own (free tier is plenty).

Caveat: an un-notarized input method still trips Gatekeeper on download — the
script's `xattr` de-quarantine is the "I trust this" step. For a frictionless
double-click installer for non-technical users, notarize instead (below).

## Distribution (signed `.pkg` + notarization)

`package.sh` builds a `.pkg` that installs `RomajiIME.app` to
`/Library/Input Methods` (system-wide). It signs + notarizes when you provide a
Developer ID identity; otherwise it makes an unsigned `.pkg` for local testing.

One-time setup (Apple Developer account):
1. Install your **Developer ID Application** and **Developer ID Installer**
   certs in the keychain (`security find-identity -v` to confirm).
2. `xcrun notarytool store-credentials romaji-ime-notary --apple-id you@example.com --team-id TEAMID --password <app-specific-password>`

Then:
```bash
CODESIGN_IDENTITY="Developer ID Application: NAME (TEAMID)" \
INSTALLER_IDENTITY="Developer ID Installer: NAME (TEAMID)" \
NOTARY_PROFILE=romaji-ime-notary \
platform/macos/package.sh        # -> platform/macos/build/RomajiIME.pkg
```
Hardened Runtime is used (no App Sandbox), which allows the cloud-AI network
call and satisfies notarization. CI can do this too — see
`.github/workflows/release.yml` (gated on repo secrets).

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
