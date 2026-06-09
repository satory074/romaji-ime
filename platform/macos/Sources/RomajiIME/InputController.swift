import Carbon  // IsSecureEventInputEnabled
import Cocoa
import InputMethodKit

/// One controller per text-input client. IMKServer creates these.
///
/// `@objc(InputController)` pins the Objective-C runtime name so it matches
/// `InputMethodServerControllerClass = InputController` in Info.plist
/// (IMKServer instantiates the class via `NSClassFromString`).
@objc(InputController)
final class InputController: IMKInputController {
    private var session: EngineSession?
    /// True while a cloud-AI conversion is in flight (keys are swallowed so the
    /// session is only touched on the main thread).
    private var converting = false

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        if session == nil {
            session = SharedEngine.shared.newSession()
        }
    }

    override func deactivateServer(_ sender: Any!) {
        if let client = sender as? IMKTextInput {
            commitCurrent(to: client)
        }
        super.deactivateServer(sender)
    }

    /// Called when the system wants any in-progress composition finalized.
    override func commitComposition(_ sender: Any!) {
        if let client = sender as? IMKTextInput {
            commitCurrent(to: client)
        }
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event = event, event.type == .keyDown else { return false }
        guard let client = sender as? IMKTextInput else { return false }
        if session == nil { session = SharedEngine.shared.newSession() }
        guard let session = session else { return false }

        // Swallow keys while a conversion is resolving (brief, ~1s).
        if converting { return true }

        let (sym, mods) = Self.translate(event)
        if sym == 0 { return false }

        // Space triggers cloud-AI conversion — the headline feature — when
        // composing. begin returns 0 if AI is unavailable or we're already
        // showing candidates, in which case we fall through to normal handling
        // (which cycles candidates / commits local kana).
        let isShortcut = mods & ((1 << 2) | (1 << 3)) != 0
        if sym == 0x20 && !isShortcut && !Self.isSecureInput() {
            let (before, after) = Self.surroundingContext(client)
            let id = session.beginAiConvert(contextBefore: before, contextAfter: after)
            if id != 0 {
                startConverting(reqId: id, client: client)
                return true
            }
        }

        let flags = session.processKey(sym: sym, mods: mods)
        // The C macros import as Int32; compare against the UInt32 flags.
        if flags & UInt32(RIME_CONSUMED) == 0 {
            // Not ours: let the client handle it (e.g. literal space, arrows).
            return false
        }
        applyResult(session: session, client: client)
        return true
    }

    /// Insert any committed text and refresh the marked (preedit) text.
    private func applyResult(session: EngineSession, client: IMKTextInput) {
        let commit = session.commitText()
        if !commit.isEmpty {
            client.insertText(commit, replacementRange: Self.noRange)
        }
        updateMarkedText(session.preedit(), client: client)
    }

    /// Poll the in-flight conversion on the main thread until it resolves. Only
    /// the HTTP call runs off-thread (inside the engine); the session is only
    /// ever touched here on main.
    private func startConverting(reqId: UInt64, client: IMKTextInput) {
        converting = true
        let start = Date()
        func poll() {
            guard converting, let session = session else { return }
            switch session.pollAiResult(reqId) {
            case 1:  // ready: candidates populated, preedit = top candidate
                converting = false
                updateMarkedText(session.preedit(), client: client)
            case -1:  // error: stay composing, leave the local kana visible
                converting = false
                updateMarkedText(session.preedit(), client: client)
            default:  // pending
                if Date().timeIntervalSince(start) > 5.0 {
                    converting = false
                    updateMarkedText(session.preedit(), client: client)
                } else {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.03, execute: poll)
                }
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.03, execute: poll)
    }

    // MARK: - Helpers

    private func commitCurrent(to client: IMKTextInput) {
        guard let session = session else { return }
        let preedit = session.preedit()
        if !preedit.isEmpty {
            client.insertText(preedit, replacementRange: Self.noRange)
        }
        session.reset()
        updateMarkedText("", client: client)
    }

    private func updateMarkedText(_ text: String, client: IMKTextInput) {
        let length = (text as NSString).length
        let selection = NSRange(location: length, length: 0)
        if text.isEmpty {
            client.setMarkedText("", selectionRange: selection, replacementRange: Self.noRange)
        } else {
            let attributed = NSAttributedString(
                string: text,
                attributes: [.underlineStyle: NSUnderlineStyle.single.rawValue]
            )
            client.setMarkedText(attributed, selectionRange: selection, replacementRange: Self.noRange)
        }
    }

    private static let noRange = NSRange(location: NSNotFound, length: 0)

    /// Translate a native key event into the engine's platform-neutral
    /// (keysym, modifiers). Keysyms follow the X11/IBus convention used by
    /// `ime_engine::key`.
    static func translate(_ event: NSEvent) -> (UInt32, UInt32) {
        var mods: UInt32 = 0
        let f = event.modifierFlags
        if f.contains(.shift) { mods |= 1 }          // SHIFT   (bit 0)
        if f.contains(.control) { mods |= 1 << 2 }   // CONTROL (bit 2)
        if f.contains(.option) { mods |= 1 << 3 }    // ALT     (bit 3)
        if f.contains(.command) { mods |= 1 << 2 }   // ⌘ -> treat like control (not text)

        switch event.keyCode {
        case 51: return (0xFF08, mods)       // Backspace (Delete)
        case 36, 76: return (0xFF0D, mods)   // Return / keypad Enter
        case 53: return (0xFF1B, mods)       // Escape
        case 49: return (0x20, mods)         // Space
        case 48: return (0xFF09, mods)       // Tab
        case 123: return (0xFF51, mods)      // Left
        case 124: return (0xFF53, mods)      // Right
        case 125: return (0xFF54, mods)      // Down
        case 126: return (0xFF52, mods)      // Up
        default: break
        }

        // Printable: use the base (modifier-independent) character, lowercased so
        // Shift doesn't break romaji lookups.
        if let chars = event.charactersIgnoringModifiers?.lowercased(),
           let scalar = chars.unicodeScalars.first {
            let v = scalar.value
            if (0x21...0x7E).contains(v) {
                return (v, mods)
            }
        }
        return (0, mods)
    }

    /// Best-effort surrounding document text for AI context. Many apps don't
    /// expose it, so this fails soft to ("", "").
    static func surroundingContext(_ client: IMKTextInput) -> (String, String) {
        let sel = client.selectedRange()
        guard sel.location != NSNotFound else { return ("", "") }
        var before = ""
        let beforeLen = min(20, sel.location)
        if beforeLen > 0,
           let attr = client.attributedSubstring(
               from: NSRange(location: sel.location - beforeLen, length: beforeLen)) {
            before = attr.string
        }
        var after = ""
        if let attr = client.attributedSubstring(from: NSRange(location: sel.location, length: 20)) {
            after = attr.string
        }
        return (before, after)
    }

    /// True when macOS secure input is active (password fields, etc). We never
    /// send keystrokes to a cloud LLM in that case.
    static func isSecureInput() -> Bool {
        IsSecureEventInputEnabled()
    }
}
