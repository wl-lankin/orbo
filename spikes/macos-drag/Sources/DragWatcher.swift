import AppKit

/// Detects file drags system-wide without any permission: polls the mouse button state
/// and watches the change count of the drag pasteboard, which the drag source writes on drag start.
final class DragWatcher {
    enum Trigger: String { case key = "Taste", shake = "Schütteln" }

    var onTrigger: ((NSPoint, Trigger, [URL]?, TimeInterval) -> Void)?
    var shouldCancel: ((NSPoint) -> Bool)?
    var onCancel: (() -> Void)?
    var onRelease: (() -> Void)?

    private let pasteboard = NSPasteboard(name: .drag)
    private var timer: Timer?
    private var fastMode = false
    private var baseline = 0
    private var pressedAt: TimeInterval = 0
    private var isFileDrag = false
    private var isOtherDrag = false
    private var triggered = false
    private var needsKeyRelease = false
    private var shake = ShakeDetector()

    func start() {
        baseline = pasteboard.changeCount
        schedule(1.0 / 30)
    }

    private func schedule(_ interval: TimeInterval) {
        timer?.invalidate()
        let t = Timer(timeInterval: interval, repeats: true) { [weak self] _ in self?.tick() }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    private func tick() {
        let buttonDown = NSEvent.pressedMouseButtons & 1 != 0
        let t = now()

        if !fastMode {
            // Baseline only moves while the button is up, so a drag that starts between two idle ticks is still caught.
            guard buttonDown else { baseline = pasteboard.changeCount; return }
            fastMode = true
            pressedAt = t
            isFileDrag = false
            isOtherDrag = false
            triggered = false
            needsKeyRelease = false
            shake.reset()
            schedule(1.0 / 120)
            return
        }

        if !buttonDown {
            fastMode = false
            let wasTriggered = triggered
            triggered = false
            baseline = pasteboard.changeCount
            schedule(1.0 / 30)
            if wasTriggered { onRelease?() }
            return
        }

        if isOtherDrag { return }
        let point = NSEvent.mouseLocation

        if !isFileDrag {
            guard pasteboard.changeCount != baseline else { return }
            let types = Settings.shared.readEarly ? pasteboard.types : nil
            // The source clears the pasteboard (bumping changeCount) before writing, so an empty list means "not yet".
            if let types, types.isEmpty { return }
            if let types, !types.contains(.fileURL) {
                isOtherDrag = true
                Log.write("Drag ohne Dateien erkannt (\(types.prefix(3).map(\.rawValue).joined(separator: ", "))), ignoriert")
                return
            }
            isFileDrag = true
            let typeInfo = types.map { "\($0.count) Typen lesbar" } ?? "Typen nicht gelesen"
            Log.write("Datei-Drag erkannt \(millis(t - pressedAt)) nach Mausklick · \(typeInfo)")
        }

        let keyDown = NSEvent.modifierFlags.contains(Settings.shared.triggerKey)
        if !keyDown { needsKeyRelease = false }

        if triggered {
            if shouldCancel?(point) == true {
                triggered = false
                needsKeyRelease = keyDown
                shake.reset()
                Log.write("Ring abgebrochen (weggezogen), Drag läuft normal weiter")
                onCancel?()
            }
            return
        }

        var trigger: Trigger?
        if keyDown && !needsKeyRelease { trigger = .key }
        if trigger == nil, Settings.shared.shakeEnabled, shake.feed(point, t) { trigger = .shake }
        guard let trigger else { return }

        triggered = true
        let urls = Settings.shared.readEarly ? readURLs() : nil
        Log.write("Auslöser: \(trigger.rawValue) · \(urls.map { "\($0.count) Datei(en) früh gelesen" } ?? "Dateien noch unbekannt")")
        onTrigger?(point, trigger, urls, t)
    }

    private func readURLs() -> [URL]? {
        let objects = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
        guard let urls = objects as? [URL], !urls.isEmpty else { return nil }
        return urls
    }
}

struct ShakeDetector {
    private var lastX: CGFloat?
    private var direction: CGFloat = 0
    private var turnX: CGFloat = 0
    private var turns: [TimeInterval] = []

    mutating func reset() {
        lastX = nil
        direction = 0
        turns = []
    }

    mutating func feed(_ p: NSPoint, _ t: TimeInterval) -> Bool {
        guard let lx = lastX else { lastX = p.x; turnX = p.x; return false }
        let dx = p.x - lx
        lastX = p.x
        guard abs(dx) >= 1.5 else { return false }
        let d: CGFloat = dx > 0 ? 1 : -1
        if direction != 0 && d != direction {
            if abs(p.x - turnX) > 18 { turns.append(t) }
            turnX = p.x
        }
        direction = d
        turns.removeAll { t - $0 > 0.7 }
        if turns.count >= 3 { turns = []; return true }
        return false
    }
}
