import Foundation

@MainActor final class GameSceneMonitor {
    private var visibility = GameplayVisibility()
    private var path: URL?
    private var offset: UInt64 = 0
    private var pending = Data()
    private var nextScan = Date.distantPast
    var isInMatch: Bool {
        refresh()
        return visibility.isInMatch
    }
    private func refresh() {
        if Date() >= nextScan {
            nextScan = Date().addingTimeInterval(2)
            let root = URL(fileURLWithPath: "/Applications/Hearthstone/Logs", isDirectory: true)
            let sessions = (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: [.creationDateKey], options: [.skipsHiddenFiles])) ?? []
            let newest = sessions.filter { $0.lastPathComponent.hasPrefix("Hearthstone_") }.sorted { $0.lastPathComponent > $1.lastPathComponent }.first?.appendingPathComponent("LoadingScreen.log")
            if newest != path { path = newest; offset = 0; pending = Data(); visibility = GameplayVisibility() }
        }
        guard let path, let handle = try? FileHandle(forReadingFrom: path) else { visibility = GameplayVisibility(); return }
        defer { try? handle.close() }
        do {
            let size = try handle.seekToEnd()
            if size < offset { offset = 0; pending = Data(); visibility = GameplayVisibility() }
            try handle.seek(toOffset: offset)
            let data = try handle.readToEnd() ?? Data()
            offset += UInt64(data.count)
            pending.append(data)
            while let newline = pending.firstIndex(of: 10) {
                visibility.consume(String(decoding: pending[..<newline], as: UTF8.self))
                pending.removeSubrange(...newline)
            }
        } catch { visibility = GameplayVisibility() }
    }
}
