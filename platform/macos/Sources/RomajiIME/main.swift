import Cocoa
import InputMethodKit

// Entry point for the input-method server process.
//
// macOS input methods run as background agents (LSBackgroundOnly). We create one
// IMKServer whose connection name MUST match `InputMethodConnectionName` in
// Info.plist (the convention is `<bundle id>_Connection`). IMKServer then spawns
// one `InputController` per text-input client that connects.

let bundleID = Bundle.main.bundleIdentifier ?? "com.satory074.inputmethod.RomajiIME"
let connectionName =
    (Bundle.main.infoDictionary?["InputMethodConnectionName"] as? String)
    ?? "\(bundleID)_Connection"

// Keep a strong reference alive for the whole process lifetime.
let server = IMKServer(name: connectionName, bundleIdentifier: bundleID)
_ = server

// Touch the engine's C ABI immediately so a linker/ABI mismatch fails loudly at
// startup (visible in Console.app) rather than silently on first keystroke.
NSLog("RomajiIME: IMKServer started (connection=%@, engine ABI=%u)",
      connectionName, rime_abi_version())

let app = NSApplication.shared
app.run()
