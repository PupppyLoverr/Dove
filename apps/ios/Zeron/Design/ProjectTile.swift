import UIKit

/// The desktop's project monogram: a rounded tile in the project's tone
/// (fill @ 0.08, letter @ 0.85, mono medium) — `ui/src/shell/project_icon.rs`.
enum ProjectTile {
    private struct CacheKey: Hashable {
        let letter: String
        let colorIndex: Int
        let side: CGFloat
        let scale: CGFloat
    }

    private static var cache: [CacheKey: UIImage] = [:]

    static func letter(_ name: String) -> String {
        String(name.trimmingCharacters(in: .whitespaces).first ?? "?").uppercased()
    }

    /// The tile as an image (composer chips), light/dark aware.
    static func image(name: String, colorIndex: Int, side: CGFloat = 14) -> UIImage {
        let count = Palette.projectDots.count
        let normalizedColorIndex = (colorIndex % count + count) % count
        let tileLetter = letter(name)
        let scale = UITraitCollection.current.displayScale > 0 ? UITraitCollection.current.displayScale : 3
        let key = CacheKey(letter: tileLetter, colorIndex: normalizedColorIndex, side: side, scale: scale)
        if let cached = cache[key] { return cached }

        let tone = Palette.projectDots[normalizedColorIndex]
        let asset = UIImageAsset()
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        for style in [UIUserInterfaceStyle.light, .dark] {
            let traits = UITraitCollection { $0.userInterfaceStyle = style; $0.displayScale = scale }
            let t = tone.resolvedColor(with: traits)
            let img = UIGraphicsImageRenderer(size: CGSize(width: side, height: side), format: format).image { _ in
                let k = side / 13
                t.withAlphaComponent(0.08).setFill()
                UIBezierPath(roundedRect: CGRect(x: 0, y: 0, width: side, height: side), cornerRadius: 3 * k).fill()
                let font = Fonts.ui(.monoMedium, 9 * k)
                let s = NSAttributedString(string: tileLetter, attributes: [.font: font, .foregroundColor: t.withAlphaComponent(0.85)])
                let size = s.size()
                s.draw(at: CGPoint(x: (side - size.width) / 2, y: side / 2 - (font.ascender - font.capHeight / 2)))
            }.withRenderingMode(.alwaysOriginal)
            asset.register(img, with: traits)
        }
        let image = asset.image(with: UITraitCollection {
            $0.userInterfaceStyle = UITraitCollection.current.userInterfaceStyle
            $0.displayScale = scale
        })
        cache[key] = image
        return image
    }
}

/// The desktop's `git-branch.svg` (template).
enum BranchIcon {
    static let image: UIImage? = UIImage(named: "tool-git-branch")
    private static var cache: [CGFloat: UIImage] = [:]

    /// Rasterized at `side` points (buttons size images intrinsically).
    static func sized(_ side: CGFloat = 13) -> UIImage? {
        guard let image else { return nil }
        if let cached = cache[side] { return cached }
        let sizedImage = UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { _ in
            image.draw(in: CGRect(x: 0, y: 0, width: side, height: side))
        }.withRenderingMode(.alwaysTemplate)
        cache[side] = sizedImage
        return sizedImage
    }
}
