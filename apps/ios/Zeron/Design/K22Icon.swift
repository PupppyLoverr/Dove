import UIKit

/// Template images from `Assets.xcassets/K22`, drawn from the K22 24-grid.
/// Call sites that still pass an SF Symbol name go through `resolve`, which
/// uses the asset when §4 of the icon map lists one and leaves the others
/// (share sheet, pin, keyboard, file-type symbols) as SF Symbols.
enum K22Icon {
    static let sfToAsset: [String: String] = [
        "archivebox": "archive-minimalistic",
        "archivebox.fill": "archive-minimalistic",
        "arrow.down": "arrow-down",
        "arrow.turn.down.right": "arrow-turn-down-right",
        "arrow.turn.left.up": "arrow-turn-left-up",
        "arrow.up": "arrow-up",
        "arrow.up.arrow.down": "sort-vertical",
        "arrow.up.circle.fill": "send",
        "arrow.up.right": "arrow-up-right",
        "camera": "camera",
        "character.cursor.ibeam": "text-cursor",
        "checkmark": "check",
        "chevron.down": "alt-arrow-down",
        "chevron.left": "alt-arrow-left",
        "chevron.right": "alt-arrow-right",
        "clock": "clock-circle",
        "desktopcomputer": "monitor",
        "doc.on.clipboard": "clipboard",
        "doc.on.doc": "copy",
        "ellipsis": "more-horizontal",
        "folder": "folder",
        "folder.badge.plus": "folder-add",
        "folder.fill": "folder",
        "gearshape": "settings",
        "link": "link",
        "magnifyingglass": "magnifer",
        "mic": "microphone",
        "pencil": "pen",
        "photo.on.rectangle": "gallery",
        "plus": "plus",
        "rectangle.compress.vertical": "fold-vertical",
        "safari": "compass",
        "sparkle": "sparkle",
        "square.on.square": "copy",
        "star.fill": "star-bold",
        "stop.circle": "stop-circle",
        "text.line.last.and.arrowtriangle.forward": "queue-send",
        "trash": "trash-bin-minimalistic",
        "tray": "tray",
        "tray.and.arrow.up": "upload",
        "tray.and.arrow.up.fill": "upload",
        "waveform": "waveform",
        "xmark": "close",
    ]

    static func image(_ name: String, pointSize: CGFloat? = nil) -> UIImage? {
        guard let base = UIImage(named: name)?.withRenderingMode(.alwaysTemplate) else { return nil }
        guard let pointSize, pointSize > 0 else { return base }
        let format = UIGraphicsImageRendererFormat()
        format.opaque = false
        format.scale = 0
        let side = CGSize(width: pointSize, height: pointSize)
        let drawn = UIGraphicsImageRenderer(size: side, format: format).image { _ in
            base.draw(in: CGRect(origin: .zero, size: side))
        }
        return drawn.withRenderingMode(.alwaysTemplate)
    }

    static func resolve(systemName: String, pointSize: CGFloat? = nil, weight: UIImage.SymbolWeight = .medium) -> UIImage? {
        if let asset = sfToAsset[systemName] {
            return image(asset, pointSize: pointSize)
        }
        if let pointSize {
            return UIImage(systemName: systemName, withConfiguration: UIImage.SymbolConfiguration(pointSize: pointSize, weight: weight))
        }
        return UIImage(systemName: systemName)
    }
}
