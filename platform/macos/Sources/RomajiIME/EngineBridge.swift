import Foundation

/// One process-global engine, shared by all input sessions.
final class SharedEngine {
    static let shared = SharedEngine()
    private let engine: OpaquePointer?

    /// Behaviour settings read from config at startup.
    let autoConvertEnabled: Bool
    let autoConvertDelayMs: Int

    private init() {
        // Pass a per-user data dir so the engine can read cloud-AI settings from
        // {dataDir}/config.json (env vars aren't available to a launchd-launched
        // IME). The engine reads it synchronously here, so config.json must exist
        // before first activation; add it then re-select the input source.
        let fm = FileManager.default
        let dataDir = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("RomajiIME")
        if let dir = dataDir {
            try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        }
        if let path = dataDir?.path {
            engine = path.withCString { rime_engine_new(nil, $0) }
        } else {
            engine = rime_engine_new(nil, nil)
        }
        if let e = engine {
            let hasAi = rime_engine_has_ai(e)
            autoConvertEnabled = rime_auto_convert_enabled(e)
            autoConvertDelayMs = Int(rime_auto_convert_delay_ms(e))
            NSLog("RomajiIME: engine ready, cloud-AI configured = %@", hasAi ? "YES" : "NO")
            DebugLog.log(
                "engine ready: dataDir=\(dataDir?.path ?? "nil") hasAI=\(hasAi) "
                    + "autoConvert=\(autoConvertEnabled) delayMs=\(autoConvertDelayMs)")
        } else {
            autoConvertEnabled = true
            autoConvertDelayMs = 500
            NSLog("RomajiIME: rime_engine_new returned NULL")
            DebugLog.log("engine init FAILED (rime_engine_new returned NULL)")
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

    func candidateCount() -> Int {
        rime_get_candidate_count(ptr)
    }

    func candidate(_ index: Int) -> String {
        guard let p = rime_get_candidate_text(ptr, index) else { return "" }
        return String(cString: p)
    }

    func highlighted() -> Int {
        rime_get_highlighted_index(ptr)
    }

    /// Start an async cloud-AI conversion; returns a request id, or 0 if
    /// unavailable (no converter, nothing composing, or candidates already shown).
    func beginAiConvert(contextBefore: String, contextAfter: String) -> UInt64 {
        rime_begin_ai_convert(ptr, contextBefore, contextAfter)
    }

    /// Poll: 0 = pending, 1 = ready (preedit/candidates updated), -1 = error.
    func pollAiResult(_ reqId: UInt64) -> Int32 {
        rime_poll_ai_result(ptr, reqId)
    }

    /// The most recent AI error message (for diagnostics).
    func lastError() -> String {
        guard let p = rime_get_last_error(ptr) else { return "" }
        return String(cString: p)
    }
}
