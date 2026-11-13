import Foundation

/// Scene transitions are logged even across reconnects; no tracker process access needed.
public struct GameplayVisibility {
    public private(set) var isInMatch = false
    public init() {}
    public mutating func consume(_ line: String) {
        if line.contains("LoadingScreen.OnScenePreUnload"), let mode = value("nextMode", in: line), mode != "GAMEPLAY" {
            isInMatch = false
        } else if line.contains("LoadingScreen.OnSceneLoaded"), let mode = value("currMode", in: line) {
            isInMatch = mode == "GAMEPLAY"
        } else if line.contains("Gameplay.Start()") {
            isInMatch = false
        } else if line.contains("MulliganManager.HandleGameStart()") {
            isInMatch = true
        }
    }
    private func value(_ key: String, in line: String) -> String? {
        guard let range = line.range(of: key + "=") else { return nil }
        return String(line[range.upperBound...].prefix { $0.isLetter || $0 == "_" }).uppercased()
    }
}
