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
    /// True while a cloud-AI conversion is in flight. A new keystroke cancels it
    /// (the session is only ever touched on the main thread).
    private var converting = false
    /// Monotonic token; bumped on every key to cancel a pending auto-convert.
    private var autoConvertToken = 0
    /// Idle delay before auto-converting once typing stops.
    private let autoConvertDelayMs = 500

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        if session == nil {
            session = SharedEngine.shared.newSession()
        }
        DebugLog.log("activateServer (session=\(session == nil ? "nil" : "ok"))")
    }

    override func deactivateServer(_ sender: Any!) {
        if let client = sender as? IMKTextInput {
            commitCurrent(to: client)
        }
        CandidateWindow.shared.hide()
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

        // A new keystroke cancels a pending auto-convert and any in-flight
        // conversion, so typing is never blocked.
        autoConvertToken &+= 1
        converting = false

        let (sym, mods) = Self.translate(event)
        DebugLog.log("handle keyCode=\(event.keyCode) -> sym=0x\(String(sym, radix: 16)) mods=\(mods)")
        if sym == 0 { return false }

        // Space triggers immediate AI conversion while composing (auto-convert on
        // pause does too). Enter does NOT convert — it commits as-is (handled by
        // the engine). begin returns 0 when AI is unavailable or candidates are
        // already shown, so we fall through to normal handling.
        let isShortcut = mods & ((1 << 2) | (1 << 3)) != 0
        if sym == 0x20 && !isShortcut && !Self.isSecureInput() {
            let (before, after) = Self.surroundingContext(client)
            let id = session.beginAiConvert(contextBefore: before, contextAfter: after)
            DebugLog.log("Space -> beginAiConvert id=\(id)")
            if id != 0 {
                startConverting(reqId: id, client: client)
                return true
            }
        }

        let flags = session.processKey(sym: sym, mods: mods)
        DebugLog.log("processKey flags=\(flags) preedit='\(session.preedit())' commit='\(session.commitText())'")
        // The C macros import as Int32; compare against the UInt32 flags.
        if flags & UInt32(RIME_CONSUMED) == 0 {
            // Not ours: let the client handle it (e.g. literal space, arrows).
            return false
        }
        applyResult(session: session, client: client)

        // Auto-convert: after a brief pause once typing stops (no Space needed).
        if !Self.isSecureInput() {
            scheduleAutoConvert(client: client)
        }
        return true
    }

    /// After `autoConvertDelayMs` of no further keys, convert the current
    /// composition automatically. Cancelled if another key arrives (token check)
    /// or if AI is unavailable / nothing is composing (begin returns 0).
    private func scheduleAutoConvert(client: IMKTextInput) {
        let token = autoConvertToken
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(autoConvertDelayMs)) { [weak self] in
            guard let self = self, token == self.autoConvertToken, !self.converting else { return }
            guard let session = self.session, !session.preedit().isEmpty else { return }
            let (before, after) = Self.surroundingContext(client)
            let id = session.beginAiConvert(contextBefore: before, contextAfter: after)
            DebugLog.log("auto-convert -> beginAiConvert id=\(id)")
            if id != 0 { self.startConverting(reqId: id, client: client) }
        }
    }

    /// Insert any committed text and refresh the marked (preedit) text.
    private func applyResult(session: EngineSession, client: IMKTextInput) {
        let commit = session.commitText()
        if !commit.isEmpty {
            client.insertText(commit, replacementRange: Self.noRange)
        }
        render(session, client: client)
    }

    /// Reflect the session state: update the inline marked text AND the candidate
    /// list window (shown below the caret, hidden when there are no candidates).
    private func render(_ session: EngineSession, client: IMKTextInput) {
        updateMarkedText(session.preedit(), client: client)
        let count = session.candidateCount()
        if count > 0 {
            var list: [String] = []
            list.reserveCapacity(count)
            for i in 0..<count { list.append(session.candidate(i)) }
            CandidateWindow.shared.show(
                candidates: list, highlighted: session.highlighted(), caret: caretRect(client))
        } else {
            CandidateWindow.shared.hide()
        }
    }

    /// The caret rectangle in screen coordinates (for positioning the candidate
    /// window). Apps that don't report it yield .zero, handled by the window.
    private func caretRect(_ client: IMKTextInput) -> NSRect {
        var rect = NSRect.zero
        _ = client.attributes(forCharacterIndex: 0, lineHeightRectangle: &rect)
        return rect
    }

    /// Poll the in-flight conversion on the main thread until it resolves. Only
    /// the HTTP call runs off-thread (inside the engine); the session is only
    /// ever touched here on main.
    private func startConverting(reqId: UInt64, client: IMKTextInput) {
        converting = true
        let start = Date()
        // Immediate feedback while the (~0.7s) round trip is in flight.
        CandidateWindow.shared.showStatus("変換中…", caret: caretRect(client))
        func poll() {
            guard converting, let session = session else { return }
            switch session.pollAiResult(reqId) {
            case 1:  // final: full candidate list ready
                converting = false
                DebugLog.log("AI ready -> '\(session.preedit())' (\(session.candidateCount()) candidates)")
                render(session, client: client)
            case 2:  // streaming: partial candidates — show them and keep polling
                render(session, client: client)
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.03, execute: poll)
            case -1:  // error: stay composing, leave the romaji/kana visible
                converting = false
                DebugLog.log("AI error -> fallback. detail: \(session.lastError())")
                render(session, client: client)
            default:  // pending
                if Date().timeIntervalSince(start) > 8.0 {
                    converting = false
                    render(session, client: client)
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
        render(session, client: client)  // empty -> clears marked text + hides candidates
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

        // Printable: use the ACTUALLY produced character so Shift yields symbols
        // (Shift+1 -> "!", Shift+/ -> "?"), but lowercase ASCII letters so romaji
        // stays case-insensitive. Ctrl/Alt combos are shortcuts, not text.
        if mods & ((1 << 2) | (1 << 3)) == 0,
           let scalar = event.characters?.unicodeScalars.first {
            var v = scalar.value
            if (0x41...0x5A).contains(v) {
                v += 0x20 // 'A'..'Z' -> 'a'..'z'
            }
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
