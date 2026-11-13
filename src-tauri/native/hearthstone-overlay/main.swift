import AppKit
import CoreGraphics

private let referenceButtonHeight: CGFloat = 30

@main @MainActor final class OverlayApp: NSObject, NSApplicationDelegate {
    private let scene = GameSceneMonitor()
    private var panel: TopLeftPanel!
    private var contentView: DisconnectButton!
    private var timer: Timer?
    static func main() {
        let app = NSApplication.shared
        let delegate = OverlayApp()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        panel = TopLeftPanel(contentRect: .init(x: 0, y: 0, width: 104, height: referenceButtonHeight), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isOpaque = false; panel.backgroundColor = .clear; panel.hasShadow = false
        panel.level = .floating; panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        contentView = DisconnectButton(frame: .init(x: 0, y: 0, width: 104, height: referenceButtonHeight))
        contentView.autoresizingMask = [.width, .height]
        contentView.onClick = {
            FileHandle.standardOutput.write(Data("skip\n".utf8))
        }
        panel.contentView = contentView
        timer = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.poll() }
        }
        DispatchQueue.global().async {
            while let line = readLine() {
                DispatchQueue.main.async {
                    guard let data = line.data(using: .utf8),
                          let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
                    self.contentView.busy = false
                    self.contentView.title = response["ok"] as? Bool == true ? "已拔线" : "拔线失败"
                    self.contentView.toolTip = response["message"] as? String
                    self.contentView.needsDisplay = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        guard !self.contentView.busy else { return }
                        self.contentView.title = "一键拔线"; self.contentView.needsDisplay = true
                    }
                }
            }
            // EOF means the owning Clash Verge process exited, including crashes.
            DispatchQueue.main.async { NSApp.terminate(nil) }
        }
    }
    func applicationWillTerminate(_ notification: Notification) { timer?.invalidate() }
    private var lastShown: Bool?
    private func poll() {
        let client = NSRunningApplication.runningApplications(withBundleIdentifier: "unity.Blizzard Entertainment.Hearthstone")
            .first { !$0.isTerminated }
        let foreground = NSWorkspace.shared.frontmostApplication?.bundleIdentifier
        guard let client,
              foreground == "unity.Blizzard Entertainment.Hearthstone" || foreground == "net.hearthsim.hstracker",
              scene.isInMatch,
              let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]],
              let primary = NSScreen.screens.first else { hide(); return }
        let frames = list.compactMap { info -> CGRect? in
            guard (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == client.processIdentifier,
                  (info[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
                  let raw = info[kCGWindowBounds as String] as? [String: Any],
                  let rect = CGRect(dictionaryRepresentation: raw as CFDictionary), rect.width > 300, rect.height > 200 else { return nil }
            return rect
        }
        guard let rect = frames.max(by: { $0.width * $0.height < $1.width * $1.height }) else { hide(); return }
        let rawWindow = CGRect(x: rect.minX, y: primary.frame.maxY - rect.maxY, width: rect.width, height: rect.height)
        let fullscreen = NSScreen.screens.contains { screen in
            abs(screen.frame.minX - rawWindow.minX) < 2 && abs(screen.frame.minY - rawWindow.minY) < 2 && abs(screen.frame.width - rawWindow.width) < 2 && abs(screen.frame.height - rawWindow.height) < 2
        }
        // Content height controls scale; the untrimmed window controls the top edge.
        let content = fullscreen ? rawWindow : NSWindow.contentRect(forFrameRect: rawWindow, styleMask: [.titled])
        guard content.height.isFinite, content.height > 0 else { hide(); return }
        let scale = content.height / 1080
        contentView.scale = scale
        let height = referenceButtonHeight * scale
        let frame = CGRect(x: rawWindow.minX + 8 * scale, y: rawWindow.maxY - height, width: 104 * scale, height: height)
        if frame != panel.frame { panel.setFrame(frame, display: true) }
        if !panel.isVisible { panel.orderFrontRegardless() }
        if lastShown != true {
            panel.displayIfNeeded()
            let actual = serverFrame(primaryTop: primary.frame.maxY).map { NSStringFromRect($0) } ?? "unavailable"
            let screens = NSScreen.screens.map { NSStringFromRect($0.frame) }.joined(separator: ",")
            log("shown: frame=\(NSStringFromRect(frame)) top=\(rawWindow.maxY) height=\(height) actual=\(actual) screens=\(screens)")
            lastShown = true
        }
    }
    private func hide() {
        panel.orderOut(nil)
        if lastShown != false { log("hidden"); lastShown = false }
    }
    private func serverFrame(primaryTop: CGFloat) -> CGRect? {
        guard let windows = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]],
              let info = windows.first(where: { ($0[kCGWindowNumber as String] as? NSNumber)?.intValue == panel.windowNumber }),
              let raw = info[kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: raw as CFDictionary) else { return nil }
        return CGRect(x: rect.minX, y: primaryTop - rect.maxY, width: rect.width, height: rect.height)
    }
    private func log(_ message: String) {
        FileHandle.standardError.write(Data("[DisabledRacesPanel] \(message)\n".utf8))
    }

}

@MainActor private final class TopLeftPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

@MainActor private final class DisconnectButton: NSView {
    var scale: CGFloat = 1 { didSet { if scale != oldValue { needsDisplay = true } } }
    var title = "一键拔线"
    var busy = false
    var onClick: (() -> Void)?
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) {
        guard !busy else { return }
        busy = true; title = "处理中…"; needsDisplay = true; onClick?()
    }
    override func draw(_ dirtyRect: NSRect) {
        NSColor(calibratedRed: 0.10, green: 0.12, blue: 0.12, alpha: 0.95).setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 5 * scale, yRadius: 5 * scale).fill()
        let text = NSAttributedString(string: title, attributes: [
            .font: NSFont.systemFont(ofSize: 14 * scale, weight: .semibold),
            .foregroundColor: NSColor(calibratedRed: 0.48, green: 0.86, blue: 0.39, alpha: 1)
        ])
        let size = text.size()
        text.draw(at: .init(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2))
    }
}
