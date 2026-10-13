import Foundation

enum TranscriptSettings {
    private static let key = "transcriptCompact"
    static let didChange = Notification.Name("TranscriptSettings.didChange")

    static var compact: Bool {
        get { UserDefaults.standard.bool(forKey: key) }
        set {
            guard compact != newValue else { return }
            UserDefaults.standard.set(newValue, forKey: key)
            NotificationCenter.default.post(name: didChange, object: nil)
        }
    }
}
