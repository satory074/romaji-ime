import Foundation

/// Minimal file logger for diagnosing the installed (system-launched) IME, where
/// stderr/NSLog isn't reliably visible. Writes to
/// ~/Library/Application Support/RomajiIME/debug.log. Never logs the API key.
enum DebugLog {
    private static let url: URL? = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)
        .first?
        .appendingPathComponent("RomajiIME/debug.log")

    private static let queue = DispatchQueue(label: "com.satory074.romajiime.debuglog")

    static func log(_ message: @autoclosure () -> String) {
        guard let url = url else { return }
        let line = "\(Date()) \(message())\n"
        let path = url.path
        queue.async {
            guard let data = line.data(using: .utf8) else { return }
            if FileManager.default.fileExists(atPath: path) {
                if let handle = try? FileHandle(forWritingTo: url) {
                    defer { try? handle.close() }
                    handle.seekToEndOfFile()
                    handle.write(data)
                }
            } else {
                try? data.write(to: url)
            }
        }
    }
}
