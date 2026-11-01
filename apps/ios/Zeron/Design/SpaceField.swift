import UIKit

/// Flying through a star field: each star falls toward the viewer and draws a
/// streak as long as its speed. `warp(_:)` pushes the speed up and it settles
/// back to `cruise`. Paused off screen and when Reduce Motion is on, where the
/// stars hold still.
final class SpaceField: UIView {
    private struct Star {
        var x: CGFloat
        var y: CGFloat
        var z: CGFloat
        var size: CGFloat
    }

    /// The speed the field settles at, in depth units per second.
    var cruise: CGFloat = 0.05

    private var stars: [Star] = []
    private var speed: CGFloat = 1.1
    private var link: CADisplayLink?
    private var last: CFTimeInterval = 0

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        isOpaque = true
        isUserInteractionEnabled = false
        contentMode = .redraw
        stars = (0..<260).map { _ in Self.spawn(z: .random(in: 0.05...1)) }
    }

    required init?(coder: NSCoder) { fatalError() }

    private static func spawn(z: CGFloat) -> Star {
        // Off-axis, so nothing sits still in the middle of the screen.
        var x: CGFloat = 0
        var y: CGFloat = 0
        repeat {
            x = .random(in: -1...1)
            y = .random(in: -1...1)
        } while x * x + y * y < 0.012
        return Star(x: x, y: y, z: z, size: .random(in: 0.5...1.5))
    }

    /// Push the field to `speed`; it eases back to `cruise`.
    func warp(_ speed: CGFloat = 1.4) {
        guard !UIAccessibility.isReduceMotionEnabled else { return }
        self.speed = max(self.speed, speed)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        link?.invalidate()
        link = nil
        guard window != nil, !UIAccessibility.isReduceMotionEnabled else {
            setNeedsDisplay()
            return
        }
        last = 0
        let link = CADisplayLink(target: self, selector: #selector(step(_:)))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    @objc private func step(_ link: CADisplayLink) {
        let dt = last == 0 ? 0 : CGFloat(min(link.timestamp - last, 0.05))
        last = link.timestamp
        // A simple decay: quick off the top, long tail into the cruise.
        speed += (cruise - speed) * min(1, dt * 1.6)
        for i in stars.indices {
            stars[i].z -= speed * dt
            if stars[i].z <= 0.02 { stars[i] = Self.spawn(z: 1) }
        }
        setNeedsDisplay()
    }

    override func draw(_ rect: CGRect) {
        guard let c = UIGraphicsGetCurrentContext() else { return }
        c.setFillColor(UIColor.black.cgColor)
        c.fill(rect)
        let center = CGPoint(x: bounds.midX, y: bounds.midY)
        let scale = max(bounds.width, bounds.height) * 0.6
        c.setLineCap(.round)
        // A streak spans the depth the star crossed in about a fifth of a second.
        let trail = min(0.42, speed * 0.2)
        for s in stars {
            let head = CGPoint(x: center.x + s.x / s.z * scale, y: center.y + s.y / s.z * scale)
            guard bounds.insetBy(dx: -40, dy: -40).contains(head) else { continue }
            let tailZ = min(1, s.z + trail)
            let tail = CGPoint(x: center.x + s.x / tailZ * scale, y: center.y + s.y / tailZ * scale)
            let near = 1 - s.z
            // Fade in from the far plane, so a respawned star never pops.
            let alpha = min(1, (1 - s.z) * 3) * (0.35 + 0.65 * near)
            c.setStrokeColor(UIColor(red: 0.82, green: 0.90, blue: 1, alpha: alpha).cgColor)
            c.setLineWidth(s.size * (0.6 + near * 1.6))
            c.move(to: tail)
            c.addLine(to: head)
            c.strokePath()
        }
    }
}
