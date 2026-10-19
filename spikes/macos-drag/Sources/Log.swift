import AppKit

enum Log {
    static let url = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Logs/OrbitSpike.log")

    private static let formatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()

    static func write(_ message: String) {
        let line = "\(formatter.string(from: Date()))  \(message)\n"
        let data = Data(line.utf8)
        FileHandle.standardOutput.write(data)
        if let handle = try? FileHandle(forWritingTo: url) {
            handle.seekToEndOfFile()
            handle.write(data)
            try? handle.close()
        } else {
            try? data.write(to: url)
        }
    }
}

func now() -> TimeInterval { ProcessInfo.processInfo.systemUptime }

func millis(_ seconds: TimeInterval) -> String { String(format: "%.1f ms", seconds * 1000) }

enum RingStyle: String { case classic = "Klassisch", glass = "Glas" }

final class Settings {
    static let shared = Settings()
    var style: RingStyle = RingStyle(rawValue: UserDefaults.standard.string(forKey: "style") ?? "") ?? .classic {
        didSet { UserDefaults.standard.set(style.rawValue, forKey: "style") }
    }
    var triggerKey: NSEvent.ModifierFlags = .option
    var shakeEnabled = true
    // Reading the drag pasteboard before the drop may be restricted by pasteboard privacy; the ring falls back to draggingEntered.
    var readEarly = true
}
