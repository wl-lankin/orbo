import AppKit

enum Geometry {
    static let panelSize: CGFloat = 420
    static let outer: CGFloat = 180
    static let inner: CGFloat = 62
    static let cancelDistance: CGFloat = 300
}

final class RingPanel: NSPanel {
    init(view: NSView) {
        let s = Geometry.panelSize
        super.init(contentRect: NSRect(x: 0, y: 0, width: s, height: s),
                   styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        level = .popUpMenu
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        isReleasedWhenClosed = false
        hidesOnDeactivate = false
        animationBehavior = .none
        contentView = view
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

final class RingController {
    let view = RingView(frame: NSRect(x: 0, y: 0, width: Geometry.panelSize, height: Geometry.panelSize))
    lazy var panel = RingPanel(view: view)
    private(set) var center: NSPoint = .zero
    private(set) var visible = false
    private var shownAt: TimeInterval = 0

    init() {
        view.onDrop = { [weak self] urls, action in self?.drop(urls, action) }
        prewarm()
    }

    /// Ordering the panel in once at launch moves window-server setup cost out of the first real trigger.
    private func prewarm() {
        panel.alphaValue = 0
        panel.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
        panel.orderFrontRegardless()
        panel.orderOut(nil)
        panel.alphaValue = 1
    }

    func show(at point: NSPoint, urls: [URL]?, triggeredAt: TimeInterval) {
        let s = Geometry.panelSize
        let screen = NSScreen.screens.first { NSMouseInRect(point, $0.frame, false) } ?? NSScreen.main!
        let f = screen.frame
        let c = NSPoint(x: min(max(point.x, f.minX + s / 2), f.maxX - s / 2),
                        y: min(max(point.y, f.minY + s / 2), f.maxY - s / 2))
        center = c
        view.configure(urls: urls ?? [])
        panel.setFrame(NSRect(x: c.x - s / 2, y: c.y - s / 2, width: s, height: s), display: false)
        panel.orderFrontRegardless()
        view.animateIn()
        CATransaction.flush()
        visible = true
        shownAt = now()
        Log.write("Ring sichtbar \(millis(shownAt - triggeredAt)) nach Auslöser (Stil: \(Settings.shared.style.rawValue), Bildschirm \(Int(f.width))×\(Int(f.height)) @\(screen.backingScaleFactor)x)")
    }

    func isFar(_ p: NSPoint) -> Bool {
        visible && hypot(p.x - center.x, p.y - center.y) > Geometry.cancelDistance
    }

    func hide() {
        guard visible else { return }
        visible = false
        view.animateOut { [weak self] in
            guard let self, !self.visible else { return }
            self.panel.orderOut(nil)
        }
    }

    private func drop(_ urls: [URL], _ action: RingAction) {
        hide()
        StatusFeedback.shared.working(action.title)
        Actions.run(action, on: urls) { ok in
            StatusFeedback.shared.done(ok)
        }
    }
}

final class RingView: NSView {
    var onDrop: (([URL], RingAction) -> Void)?

    private var urls: [URL] = []
    private var actions: [RingAction] = []
    private var hot: Int?   // -1 = center
    private let stage = NSView()
    private var backdrop: NSView?
    private var lens: NSView?
    private let sectorLayer = CALayer()
    private let lensText = CATextLayer()
    private var sectorShapes: [CAShapeLayer] = []
    private var styleBuilt: RingStyle?

    private var mid: NSPoint { NSPoint(x: bounds.midX, y: bounds.midY) }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = .clear
        registerForDraggedTypes([.fileURL])
        stage.frame = bounds
        stage.wantsLayer = true
        addSubview(stage)
    }

    required init?(coder: NSCoder) { fatalError() }

    // MARK: Building

    private func buildBackground() {
        let style = Settings.shared.style
        guard styleBuilt != style else { return }
        styleBuilt = style
        stage.subviews.forEach { $0.removeFromSuperview() }

        let r = Geometry.outer, ri = Geometry.inner
        let discFrame = NSRect(x: mid.x - r, y: mid.y - r, width: 2 * r, height: 2 * r)
        let lensFrame = NSRect(x: mid.x - ri, y: mid.y - ri, width: 2 * ri, height: 2 * ri)

        switch style {
        case .glass:
            backdrop = makeGlass(frame: discFrame, radius: r)
            lens = makeGlass(frame: lensFrame, radius: ri)
        case .classic:
            backdrop = makeBlur(frame: discFrame, radius: r, tint: NSColor(srgbRed: 0.07, green: 0.08, blue: 0.14, alpha: 0.35))
            lens = makeBlur(frame: lensFrame, radius: ri, tint: NSColor(srgbRed: 0.05, green: 0.06, blue: 0.11, alpha: 0.82))
        }

        let sectors = NSView(frame: bounds)
        sectors.wantsLayer = true
        sectorLayer.frame = bounds
        sectors.layer?.addSublayer(sectorLayer)

        let lensOverlay = NSView(frame: bounds)
        lensOverlay.wantsLayer = true
        lensText.frame = CGRect(x: mid.x - 54, y: mid.y - 30, width: 108, height: 60)
        lensText.alignmentMode = .center
        lensText.isWrapped = true
        lensText.contentsScale = NSScreen.main?.backingScaleFactor ?? 2
        lensOverlay.layer?.addSublayer(lensText)

        [backdrop!, sectors, lens!, lensOverlay].forEach { stage.addSubview($0) }
    }

    private func makeGlass(frame: NSRect, radius: CGFloat) -> NSView {
        if #available(macOS 26.0, *) {
            let g = NSGlassEffectView(frame: frame)
            g.cornerRadius = radius
            return g
        }
        return makeBlur(frame: frame, radius: radius, tint: NSColor.white.withAlphaComponent(0.08))
    }

    private func makeBlur(frame: NSRect, radius: CGFloat, tint: NSColor) -> NSView {
        let v = NSVisualEffectView(frame: frame)
        v.material = .hudWindow
        v.blendingMode = .behindWindow
        v.state = .active
        v.wantsLayer = true
        v.layer?.cornerRadius = radius
        v.layer?.masksToBounds = true
        let t = NSView(frame: v.bounds)
        t.wantsLayer = true
        t.layer?.backgroundColor = tint.cgColor
        t.autoresizingMask = [.width, .height]
        v.addSubview(t)
        return v
    }

    func configure(urls newURLs: [URL]) {
        buildBackground()
        urls = newURLs
        actions = newURLs.isEmpty ? [] : Actions.forFiles(newURLs)
        hot = nil
        sectorLayer.sublayers?.forEach { $0.removeFromSuperlayer() }
        sectorShapes = []

        let n = max(actions.count, 1)
        let step = 2 * CGFloat.pi / CGFloat(n)
        let scale = NSScreen.main?.backingScaleFactor ?? 2
        let glass = Settings.shared.style == .glass

        for (i, action) in actions.enumerated() {
            let a = CGFloat(i) * step
            let shape = CAShapeLayer()
            shape.path = sectorPath(from: a - step / 2 + 0.02, to: a + step / 2 - 0.02)
            shape.fillColor = baseFill(glass)
            shape.strokeColor = NSColor.white.withAlphaComponent(glass ? 0.35 : 0.12).cgColor
            shape.lineWidth = 1
            sectorLayer.addSublayer(shape)
            sectorShapes.append(shape)

            let bandMid = (Geometry.inner + 6 + Geometry.outer - 2) / 2
            let label = CATextLayer()
            label.contentsScale = scale
            label.alignmentMode = .center
            let text = attributed(action.title, action.subtitle, color: action.color)
            label.string = text
            label.frame = centeredFrame(for: text, at: polar(bandMid, a), width: 120)
            sectorLayer.addSublayer(label)

            if i == 0 {
                let dot = CAShapeLayer()
                let dp = polar(Geometry.outer - 10, a)
                dot.path = CGPath(ellipseIn: CGRect(x: dp.x - 3, y: dp.y - 3, width: 6, height: 6), transform: nil)
                dot.fillColor = action.color.cgColor
                sectorLayer.addSublayer(dot)
            }
        }
        updateLens()
    }

    private func baseFill(_ glass: Bool) -> CGColor {
        glass ? NSColor.white.withAlphaComponent(0.06).cgColor
              : NSColor(srgbRed: 0.08, green: 0.09, blue: 0.15, alpha: 0.72).cgColor
    }

    private func attributed(_ title: String, _ sub: String, color: NSColor) -> NSAttributedString {
        let para = NSMutableParagraphStyle()
        para.alignment = .center
        let s = NSMutableAttributedString(string: title + "\n", attributes: [
            .font: NSFont.systemFont(ofSize: 16, weight: .heavy), .foregroundColor: NSColor.white, .paragraphStyle: para])
        s.append(NSAttributedString(string: sub, attributes: [
            .font: NSFont.systemFont(ofSize: 10.5, weight: .medium), .foregroundColor: NSColor.white.withAlphaComponent(0.7),
            .paragraphStyle: para]))
        return s
    }

    private func updateLens() {
        let para = NSMutableParagraphStyle()
        para.alignment = .center
        let text = NSMutableAttributedString()
        if actions.isEmpty {
            text.append(NSAttributedString(string: "Datei\nhierher ziehen", attributes: [
                .font: NSFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: NSColor.white, .paragraphStyle: para]))
        } else {
            let a = actions[(hot ?? -1) >= 0 ? hot! : 0]
            let head = hot == nil ? Actions.kind(of: urls).rawValue + (urls.count > 1 ? " ×\(urls.count)" : "") : "→ " + a.title
            let sub = hot == nil ? "Mitte: \(actions[0].title)" : a.subtitle
            text.append(NSAttributedString(string: head + "\n", attributes: [
                .font: NSFont.systemFont(ofSize: 17, weight: .heavy), .foregroundColor: hot == nil ? NSColor.white : a.color,
                .paragraphStyle: para]))
            text.append(NSAttributedString(string: sub, attributes: [
                .font: NSFont.systemFont(ofSize: 10.5, weight: .medium), .foregroundColor: NSColor.white.withAlphaComponent(0.75),
                .paragraphStyle: para]))
        }
        lensText.string = text
        lensText.frame = centeredFrame(for: text, at: mid, width: 2 * Geometry.inner - 12)

        if #available(macOS 26.0, *), let g = lens as? NSGlassEffectView {
            g.tintColor = hot == -1 && !actions.isEmpty ? actions[0].color.withAlphaComponent(0.55) : nil
        }
    }

    /// CATextLayer draws from the top of its frame, so the frame must be exactly as tall as the text to center it.
    private func centeredFrame(for text: NSAttributedString, at p: NSPoint, width: CGFloat) -> CGRect {
        let h = ceil(text.boundingRect(with: CGSize(width: width, height: 400),
                                       options: [.usesLineFragmentOrigin, .usesFontLeading]).height)
        return CGRect(x: (p.x - width / 2).rounded(), y: (p.y - h / 2).rounded(), width: width, height: h)
    }

    private func polar(_ r: CGFloat, _ a: CGFloat) -> NSPoint {
        NSPoint(x: mid.x + r * sin(a), y: mid.y + r * cos(a))
    }

    /// Angles run clockwise from north; AppKit's y axis points up, so the standard angle is pi/2 - a.
    private func sectorPath(from a0: CGFloat, to a1: CGFloat) -> CGPath {
        let p = CGMutablePath()
        p.addArc(center: mid, radius: Geometry.outer - 2, startAngle: .pi / 2 - a0, endAngle: .pi / 2 - a1, clockwise: true)
        p.addArc(center: mid, radius: Geometry.inner + 6, startAngle: .pi / 2 - a1, endAngle: .pi / 2 - a0, clockwise: false)
        p.closeSubpath()
        return p
    }

    // MARK: Hover

    private func hit(_ p: NSPoint) -> Int? {
        let dx = p.x - mid.x, dy = p.y - mid.y, d = hypot(dx, dy)
        if d < Geometry.inner { return -1 }
        guard d <= Geometry.outer + 14, !actions.isEmpty else { return nil }
        var a = atan2(dx, dy)
        if a < 0 { a += 2 * .pi }
        let step = 2 * CGFloat.pi / CGFloat(actions.count)
        return Int((a / step).rounded()) % actions.count
    }

    private func setHot(_ h: Int?) {
        guard h != hot else { return }
        hot = h
        let glass = Settings.shared.style == .glass
        CATransaction.begin()
        CATransaction.setAnimationDuration(0.12)
        for (i, shape) in sectorShapes.enumerated() {
            let isHot = i == h || (h == -1 && i == 0)
            let color = actions[i].color
            shape.fillColor = isHot ? color.withAlphaComponent(glass ? 0.38 : 0.32).cgColor : baseFill(glass)
            shape.strokeColor = isHot ? color.cgColor : NSColor.white.withAlphaComponent(glass ? 0.35 : 0.12).cgColor
            shape.lineWidth = isHot ? 1.6 : 1
        }
        CATransaction.commit()
        updateLens()
        if h != nil { NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now) }
    }

    // MARK: Animation

    func animateIn() {
        guard let layer = stage.layer else { return }
        layer.anchorPoint = CGPoint(x: 0.5, y: 0.5)
        layer.position = mid
        let spring = CASpringAnimation(keyPath: "transform.scale")
        spring.fromValue = 0.55
        spring.toValue = 1
        spring.damping = 16
        spring.stiffness = 260
        spring.mass = 1
        spring.duration = spring.settlingDuration
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0
        fade.toValue = 1
        fade.duration = 0.12
        layer.opacity = 1
        layer.add(spring, forKey: "in")
        layer.add(fade, forKey: "fade")
    }

    func animateOut(_ done: @escaping () -> Void) {
        guard let layer = stage.layer else { return done() }
        CATransaction.begin()
        CATransaction.setCompletionBlock(done)
        let scale = CABasicAnimation(keyPath: "transform.scale")
        scale.fromValue = 1
        scale.toValue = 0.3
        scale.duration = 0.18
        scale.timingFunction = CAMediaTimingFunction(name: .easeIn)
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 1
        fade.toValue = 0
        fade.duration = 0.18
        layer.opacity = 0
        layer.add(scale, forKey: "out")
        layer.add(fade, forKey: "fadeOut")
        CATransaction.commit()
    }

    // MARK: Drag destination

    private func readURLs(_ info: NSDraggingInfo) -> [URL] {
        let objects = info.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
        return objects as? [URL] ?? []
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        let u = readURLs(sender)
        if u != urls {
            Log.write("draggingEntered: \(u.count) Datei(en) (\(u.map(\.pathExtension).joined(separator: ", "))) · Ring neu aufgebaut")
            configure(urls: u)
        }
        return update(sender)
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { update(sender) }

    override func draggingExited(_ sender: NSDraggingInfo?) { setHot(nil) }

    private func update(_ sender: NSDraggingInfo) -> NSDragOperation {
        let h = hit(convert(sender.draggingLocation, from: nil))
        setHot(h)
        return h == nil || actions.isEmpty ? [] : .copy
    }

    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { hot != nil }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard let h = hot, !actions.isEmpty else { return false }
        let action = actions[h == -1 ? 0 : h]
        let files = readURLs(sender)
        Log.write("Drop auf \(h == -1 ? "Mitte" : "Feld") \(action.title) · \(files.count) Datei(en)")
        onDrop?(files, action)
        return true
    }
}
