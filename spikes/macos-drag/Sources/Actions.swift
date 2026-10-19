import AppKit
import UniformTypeIdentifiers

/// Conversions for the spike use only tools that ship with macOS (sips, avconvert, afconvert, textutil, ditto).
struct RingAction {
    let title: String
    let subtitle: String
    let color: NSColor
    let outputExtension: String
    let command: (URL, URL) -> (String, [String])
}

enum FileKind: String { case image = "Bild", video = "Video", audio = "Audio", text = "Text", other = "Datei" }

enum Actions {
    static let imageColor = NSColor(srgbRed: 0.69, green: 0.59, blue: 0.99, alpha: 1)
    static let videoColor = NSColor(srgbRed: 1.0, green: 0.50, blue: 0.58, alpha: 1)
    static let audioColor = NSColor(srgbRed: 0.36, green: 0.88, blue: 0.56, alpha: 1)
    static let textColor = NSColor(srgbRed: 0.45, green: 0.69, blue: 1.0, alpha: 1)
    static let otherColor = NSColor(srgbRed: 1.0, green: 0.72, blue: 0.47, alpha: 1)

    static func kind(of url: URL) -> FileKind {
        let type = (try? url.resourceValues(forKeys: [.contentTypeKey]))?.contentType
            ?? UTType(filenameExtension: url.pathExtension)
        guard let type else { return .other }
        if type.conforms(to: .image) { return .image }
        if type.conforms(to: .movie) || type.conforms(to: .video) { return .video }
        if type.conforms(to: .audio) { return .audio }
        if type.conforms(to: .text) || type.conforms(to: .rtf) || type.conforms(to: .html)
            || ["doc", "docx", "odt", "webarchive"].contains(url.pathExtension.lowercased()) { return .text }
        return .other
    }

    static func kind(of urls: [URL]) -> FileKind {
        let kinds = Set(urls.map(kind(of:)))
        return kinds.count == 1 ? kinds.first! : .other
    }

    static func forFiles(_ urls: [URL]) -> [RingAction] {
        let ext = urls.count == 1 ? urls[0].pathExtension.lowercased() : ""
        let list: [RingAction]
        switch kind(of: urls) {
        case .image: list = image
        case .video: list = video
        case .audio: list = audio
        case .text: list = text
        case .other: list = []
        }
        return list.filter { $0.outputExtension != ext || $0.title.contains("px") } + [zip]
    }

    private static func sips(_ format: String, _ extra: [String] = []) -> (URL, URL) -> (String, [String]) {
        { src, dst in ("/usr/bin/sips", ["-s", "format", format] + extra + [src.path, "--out", dst.path]) }
    }

    static let image: [RingAction] = [
        RingAction(title: "JPG", subtitle: "überall kompatibel", color: imageColor, outputExtension: "jpg",
                   command: sips("jpeg", ["-s", "formatOptions", "85"])),
        RingAction(title: "HEIC", subtitle: "halbe Größe", color: imageColor, outputExtension: "heic", command: sips("heic")),
        RingAction(title: "PNG", subtitle: "verlustfrei", color: imageColor, outputExtension: "png", command: sips("png")),
        RingAction(title: "1600 px", subtitle: "für Mail", color: imageColor, outputExtension: "jpg",
                   command: sips("jpeg", ["-Z", "1600", "-s", "formatOptions", "80"])),
        RingAction(title: "PDF", subtitle: "als Dokument", color: imageColor, outputExtension: "pdf", command: sips("pdf")),
    ]

    private static func avconvert(_ preset: String) -> (URL, URL) -> (String, [String]) {
        { src, dst in ("/usr/bin/avconvert", ["--preset", preset, "--source", src.path, "--output", dst.path]) }
    }

    static let video: [RingAction] = [
        RingAction(title: "MP4", subtitle: "überall abspielbar", color: videoColor, outputExtension: "mp4",
                   command: avconvert("Preset1920x1080")),
        RingAction(title: "HEVC", subtitle: "halbe Größe", color: videoColor, outputExtension: "mov",
                   command: avconvert("PresetHEVCHighestQuality")),
        RingAction(title: "720p", subtitle: "für Mail", color: videoColor, outputExtension: "mp4",
                   command: avconvert("Preset1280x720")),
        RingAction(title: "M4A", subtitle: "nur Ton", color: audioColor, outputExtension: "m4a",
                   command: avconvert("PresetAppleM4A")),
    ]

    private static func afconvert(_ file: String, _ data: String) -> (URL, URL) -> (String, [String]) {
        { src, dst in ("/usr/bin/afconvert", ["-f", file, "-d", data, src.path, dst.path]) }
    }

    static let audio: [RingAction] = [
        RingAction(title: "M4A", subtitle: "AAC, klein", color: audioColor, outputExtension: "m4a", command: afconvert("m4af", "aac")),
        RingAction(title: "ALAC", subtitle: "verlustfrei", color: audioColor, outputExtension: "m4a", command: afconvert("m4af", "alac")),
        RingAction(title: "WAV", subtitle: "unkomprimiert", color: audioColor, outputExtension: "wav", command: afconvert("WAVE", "LEI16")),
        RingAction(title: "AIFF", subtitle: "für Mac-Apps", color: audioColor, outputExtension: "aiff", command: afconvert("AIFF", "BEI16")),
    ]

    private static func textutil(_ format: String, _ ext: String) -> RingAction {
        let names = ["docx": ("DOCX", "Word"), "rtf": ("RTF", "überall lesbar"), "txt": ("TXT", "nur Text"),
                     "html": ("HTML", "Webseite"), "odt": ("ODT", "LibreOffice")]
        let (title, subtitle) = names[ext] ?? (ext.uppercased(), "")
        return RingAction(title: title, subtitle: subtitle, color: textColor, outputExtension: ext) { src, dst in
            ("/usr/bin/textutil", ["-convert", format, src.path, "-output", dst.path])
        }
    }

    static let text: [RingAction] = [
        textutil("docx", "docx"), textutil("rtf", "rtf"), textutil("txt", "txt"), textutil("html", "html"), textutil("odt", "odt"),
    ]

    static let zip = RingAction(title: "ZIP", subtitle: "packen", color: otherColor, outputExtension: "zip") { src, dst in
        ("/usr/bin/ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", src.path, dst.path])
    }

    static func outputURL(for src: URL, action: RingAction) -> URL {
        let dir = src.deletingLastPathComponent()
        var base = src.deletingPathExtension().lastPathComponent
        if action.title.contains("px") || action.title == "720p" { base += " (\(action.title))" }
        var candidate = dir.appendingPathComponent(base).appendingPathExtension(action.outputExtension)
        var n = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            candidate = dir.appendingPathComponent("\(base) \(n)").appendingPathExtension(action.outputExtension)
            n += 1
        }
        return candidate
    }

    static func run(_ action: RingAction, on urls: [URL], completion: @escaping (Bool) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async {
            var allOK = true
            for src in urls {
                let dst = outputURL(for: src, action: action)
                let (tool, args) = action.command(src, dst)
                let start = now()
                let process = Process()
                process.executableURL = URL(fileURLWithPath: tool)
                process.arguments = args
                process.standardOutput = FileHandle.nullDevice
                let errPipe = Pipe()
                process.standardError = errPipe
                do {
                    try process.run()
                    process.waitUntilExit()
                } catch {
                    Log.write("FEHLER beim Start von \(tool): \(error.localizedDescription)")
                    allOK = false
                    continue
                }
                let ok = process.terminationStatus == 0 && FileManager.default.fileExists(atPath: dst.path)
                if ok {
                    Log.write("Fertig: \(dst.lastPathComponent) in \(millis(now() - start))")
                } else {
                    let err = String(data: errPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
                    Log.write("FEHLER \(action.title) für \(src.lastPathComponent): Status \(process.terminationStatus) \(err.trimmingCharacters(in: .whitespacesAndNewlines))")
                    allOK = false
                }
            }
            DispatchQueue.main.async { completion(allOK) }
        }
    }
}
