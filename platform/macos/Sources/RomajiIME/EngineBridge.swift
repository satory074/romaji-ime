import Foundation

/// One process-global engine, shared by all input sessions.
final class SharedEngine {
    static let shared = SharedEngine()
    private let engine: OpaquePointer?

    private init() {
        // M1: no dictionary/config yet. M2+ will pass the bundle's resource dir
        // and an Application Support dir for the learning DB / settings.
        engine = rime_engine_new(nil, nil)
        if engine == nil {
            NSLog("RomajiIME: rime_engine_new returned NULL")
        }
    }

    func newSession() -> EngineSession? {
        guard let engine = engine, let ptr = rime_session_new(engine) else {
            NSLog("RomajiIME: failed to create engine session")
            return nil
        }
        return EngineSession(ptr: ptr)
    }
}

/// One input context's engine session. Wraps the opaque `RimeSession*`.
final class EngineSession {
    private let ptr: OpaquePointer
    init(ptr: OpaquePointer) { self.ptr = ptr }
    deinit { rime_session_free(ptr) }

    func processKey(sym: UInt32, mods: UInt32) -> UInt32 {
        rime_process_key(ptr, sym, mods)
    }

    func preedit() -> String {
        guard let p = rime_get_preedit(ptr) else { return "" }
        return String(cString: p)
    }

    func commitText() -> String {
        guard let p = rime_get_commit_text(ptr) else { return "" }
        return String(cString: p)
    }

    func reset() {
        rime_session_reset(ptr)
    }
}
