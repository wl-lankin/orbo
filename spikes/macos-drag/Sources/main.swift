import AppKit

final class StatusFeedback {
    static let shared = StatusFeedback()
    var item: NSStatusItem?
    private var resetWork: DispatchWorkItem?

    func set(_ title: String) {
        item?.button?.title = title
    }

    func working(_ what: String) {
        resetWork?.cancel()
        set("◎ \(what) …")
    }

    func done(_ ok: Bool) {
        set(ok ? "◎ ✓" : "◎ ✗")
        NSSound(named: ok ? "Glass" : "Basso")?.play()
        let work = DispatchWorkItem { [weak self] in self?.set("◎") }
        resetWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.5, execute: work)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private let watcher = DragWatcher()
    private var ring: RingController!
    private var statusItem: NSStatusItem!

    func applicationDidFinishLaunching(_ notification: Notification) {
        Log.write("──── OrbitSpike gestartet · macOS \(ProcessInfo.processInfo.operatingSystemVersionString) · \(NSScreen.screens.count) Bildschirm(e)")
        ring = RingController()

        watcher.onTrigger = { [weak self] point, _, urls, t in
            self?.ring.show(at: point, urls: urls, triggeredAt: t)
        }
        watcher.shouldCancel = { [weak self] p in self?.ring.isFar(p) ?? false }
        watcher.onCancel = { [weak self] in self?.ring.hide() }
        watcher.onRelease = { [weak self] in
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { self?.ring.hide() }
        }
        watcher.start()
        buildMenu()
    }

    private func buildMenu() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.title = "◎"
        StatusFeedback.shared.item = statusItem

        let menu = NSMenu()
        menu.addItem(withTitle: "Orbit Machbarkeitstest", action: nil, keyEquivalent: "")
        menu.addItem(.separator())

        let styleMenu = NSMenu()
        for style in [RingStyle.classic, .glass] {
            let item = NSMenuItem(title: style.rawValue, action: #selector(pickStyle(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = style.rawValue
            item.state = Settings.shared.style == style ? .on : .off
            styleMenu.addItem(item)
        }
        let styleItem = NSMenuItem(title: "Darstellung", action: nil, keyEquivalent: "")
        styleItem.submenu = styleMenu
        menu.addItem(styleItem)

        let keyMenu = NSMenu()
        let keys: [(String, NSEvent.ModifierFlags)] = [("⌥ Option", .option), ("⌃ Control", .control), ("⇧ Shift", .shift),
                                                      ("⌘ Command", .command), ("fn / Globus", .function)]
        for (i, (title, flag)) in keys.enumerated() {
            let item = NSMenuItem(title: title, action: #selector(pickKey(_:)), keyEquivalent: "")
            item.target = self
            item.tag = i
            item.state = Settings.shared.triggerKey == flag ? .on : .off
            keyMenu.addItem(item)
        }
        let keyItem = NSMenuItem(title: "Auslöser-Taste", action: nil, keyEquivalent: "")
        keyItem.submenu = keyMenu
        menu.addItem(keyItem)

        menu.addItem(toggle("Schütteln als Auslöser", #selector(toggleShake(_:)), Settings.shared.shakeEnabled))
        menu.addItem(toggle("Dateien früh lesen (Drag-Pasteboard)", #selector(toggleEarly(_:)), Settings.shared.readEarly))
        menu.addItem(.separator())
        let log = NSMenuItem(title: "Log öffnen", action: #selector(openLog), keyEquivalent: "l")
        log.target = self
        menu.addItem(log)
        menu.addItem(withTitle: "Beenden", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        statusItem.menu = menu
    }

    private func toggle(_ title: String, _ action: Selector, _ on: Bool) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        item.state = on ? .on : .off
        return item
    }

    private static let keyFlags: [NSEvent.ModifierFlags] = [.option, .control, .shift, .command, .function]

    @objc private func pickKey(_ sender: NSMenuItem) {
        Settings.shared.triggerKey = Self.keyFlags[sender.tag]
        sender.menu?.items.forEach { $0.state = $0 == sender ? .on : .off }
        Log.write("Auslöser-Taste: \(sender.title)")
    }

    @objc private func pickStyle(_ sender: NSMenuItem) {
        guard let raw = sender.representedObject as? String, let style = RingStyle(rawValue: raw) else { return }
        Settings.shared.style = style
        sender.menu?.items.forEach { $0.state = $0 == sender ? .on : .off }
        Log.write("Darstellung: \(style.rawValue)")
    }

    @objc private func toggleShake(_ sender: NSMenuItem) {
        Settings.shared.shakeEnabled.toggle()
        sender.state = Settings.shared.shakeEnabled ? .on : .off
    }

    @objc private func toggleEarly(_ sender: NSMenuItem) {
        Settings.shared.readEarly.toggle()
        sender.state = Settings.shared.readEarly ? .on : .off
    }

    @objc private func openLog() {
        NSWorkspace.shared.open(Log.url)
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
