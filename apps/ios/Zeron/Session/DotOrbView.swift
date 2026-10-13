import QuartzCore
import UIKit

@MainActor
final class DotOrbView: UIView {
    enum Mode: Equatable {
        case idle
        case listening
        case thinking
        case speaking
        case paused
    }

    private struct Dot {
        let x: CGFloat
        let y: CGFloat
        let z: CGFloat
        let phase: CGFloat
    }

    private struct Parameters {
        var rotation: CGFloat
        var radius: CGFloat
        var amplitude: CGFloat
        var breathing: CGFloat
        var opacity: CGFloat
    }

    private static let ink = UIColor { traits in
        traits.userInterfaceStyle == .dark ? .white : .black
    }

    private static let dots: [Dot] = {
        let count = 480
        let goldenAngle = CGFloat.pi * (3 - sqrt(5))
        return (0..<count).map { index in
            let y = 1 - 2 * (CGFloat(index) + 0.5) / CGFloat(count)
            let ring = sqrt(max(0, 1 - y * y))
            let angle = goldenAngle * CGFloat(index)
            return Dot(
                x: cos(angle) * ring,
                y: y,
                z: sin(angle) * ring,
                phase: .random(in: 0..<(2 * .pi))
            )
        }
    }()
    private static let tilt = CGFloat.pi * 18 / 180
    private static let tiltCosine = cos(tilt)
    private static let tiltSine = sin(tilt)
    private static let baseBucketOpacities: [Float] = (0..<8).map { index in
        let depth01 = CGFloat(index) / 7
        return Float(0.06 + 0.94 * CGFloat(pow(Double(depth01), 1.6)))
    }

    private let buckets: [CAShapeLayer] = (0..<8).map { _ in CAShapeLayer() }
    private let displayProxy = DisplayLinkProxy()
    private var displayLink: CADisplayLink?
    private var parameters = Parameters(rotation: 0.16, radius: 1, amplitude: 0.025, breathing: 0.012, opacity: 1)
    private var targetLevel: CGFloat = 0
    private var level: CGFloat = 0
    private var pulseEnergy: CGFloat = 0
    private var lastTimestamp: CFTimeInterval?
    private let startedAt = CACurrentMediaTime()

    var mode: Mode = .idle {
        didSet {
            guard oldValue != mode else { return }
            updateDisplayLinkFrameRate()
            if UIAccessibility.isReduceMotionEnabled {
                parameters = targetParameters(for: mode, energy: max(level, pulseEnergy))
                renderFrame(at: CACurrentMediaTime())
            } else {
                startDisplayLinkIfNeeded()
            }
        }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        parameters = targetParameters(for: .idle, energy: 0)
        isAccessibilityElement = true
        accessibilityLabel = "Voice activity"

        for bucket in buckets {
            layer.addSublayer(bucket)
        }
        updateInkColor()

        displayProxy.target = self
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(reduceMotionChanged),
            name: UIAccessibility.reduceMotionStatusDidChangeNotification,
            object: nil
        )
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit {
        MainActor.assumeIsolated {
            displayLink?.invalidate()
            NotificationCenter.default.removeObserver(self)
        }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            stopDisplayLink()
        } else if UIAccessibility.isReduceMotionEnabled {
            parameters = targetParameters(for: mode, energy: max(level, pulseEnergy))
            renderFrame(at: CACurrentMediaTime())
        } else {
            startDisplayLinkIfNeeded()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        if UIAccessibility.isReduceMotionEnabled || displayLink == nil {
            if UIAccessibility.isReduceMotionEnabled {
                parameters = targetParameters(for: mode, energy: max(level, pulseEnergy))
            }
            renderFrame(at: CACurrentMediaTime())
        }
    }

    override func traitCollectionDidChange(_ previousTraitCollection: UITraitCollection?) {
        super.traitCollectionDidChange(previousTraitCollection)
        guard
            previousTraitCollection?.userInterfaceStyle != traitCollection.userInterfaceStyle
                || previousTraitCollection?.displayScale != traitCollection.displayScale
        else { return }
        updateInkColor()
        renderFrame(at: CACurrentMediaTime())
    }

    func setLevel(_ level: CGFloat) {
        targetLevel = min(1, max(0, level))
        if UIAccessibility.isReduceMotionEnabled {
            self.level = targetLevel
            parameters = targetParameters(for: mode, energy: max(self.level, pulseEnergy))
            renderFrame(at: CACurrentMediaTime())
        }
    }

    func pulse() {
        pulseEnergy = max(pulseEnergy, 1)
        if UIAccessibility.isReduceMotionEnabled {
            parameters = targetParameters(for: mode, energy: max(level, pulseEnergy))
            renderFrame(at: CACurrentMediaTime())
        }
    }

    @objc private func reduceMotionChanged() {
        if UIAccessibility.isReduceMotionEnabled {
            stopDisplayLink()
            parameters = targetParameters(for: mode, energy: max(level, pulseEnergy))
            renderFrame(at: CACurrentMediaTime())
        } else {
            startDisplayLinkIfNeeded()
        }
    }

    fileprivate func displayTick(at timestamp: CFTimeInterval) {
        let dt = max(0, timestamp - (lastTimestamp ?? timestamp))
        lastTimestamp = timestamp
        let attack = CGFloat(1 - exp(-dt * 18))
        let release = CGFloat(1 - exp(-dt * 5))
        level += (targetLevel - level) * (targetLevel > level ? attack : release)
        pulseEnergy *= CGFloat(exp(-dt * 4))

        let target = targetParameters(for: mode, energy: max(level, pulseEnergy))
        let smoothing = CGFloat(1 - exp(-dt * 3.5))
        parameters.rotation += (target.rotation - parameters.rotation) * smoothing
        parameters.radius += (target.radius - parameters.radius) * smoothing
        parameters.amplitude += (target.amplitude - parameters.amplitude) * smoothing
        parameters.breathing += (target.breathing - parameters.breathing) * smoothing
        parameters.opacity += (target.opacity - parameters.opacity) * smoothing
        renderFrame(at: timestamp)
    }

    private func startDisplayLinkIfNeeded() {
        guard displayLink == nil, window != nil, !UIAccessibility.isReduceMotionEnabled else { return }
        lastTimestamp = nil
        let link = CADisplayLink(target: displayProxy, selector: #selector(DisplayLinkProxy.step(_:)))
        link.preferredFrameRateRange = frameRateRange
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    private var frameRateRange: CAFrameRateRange {
        switch mode {
        case .listening, .speaking:
            CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)
        case .idle, .thinking, .paused:
            CAFrameRateRange(minimum: 20, maximum: 30, preferred: 30)
        }
    }

    private func updateDisplayLinkFrameRate() {
        displayLink?.preferredFrameRateRange = frameRateRange
    }

    private func stopDisplayLink() {
        displayLink?.invalidate()
        displayLink = nil
        lastTimestamp = nil
    }

    private func targetParameters(for mode: Mode, energy: CGFloat) -> Parameters {
        switch mode {
        case .idle:
            return Parameters(rotation: 0.16, radius: 1, amplitude: 0.025, breathing: 0.012, opacity: 1)
        case .listening:
            return Parameters(rotation: 0.2, radius: 1, amplitude: 0.03 + 0.14 * energy, breathing: 0, opacity: 1)
        case .thinking:
            return Parameters(rotation: 0.42, radius: 0.97, amplitude: 0.05, breathing: 0, opacity: 1)
        case .speaking:
            return Parameters(rotation: 0.22, radius: 1, amplitude: 0.04 + 0.16 * energy, breathing: 0.02, opacity: 1)
        case .paused:
            return Parameters(rotation: 0.06, radius: 0.94, amplitude: 0.008, breathing: 0, opacity: 0.35)
        }
    }

    private func renderFrame(at timestamp: CFTimeInterval) {
        guard bounds.width > 0, bounds.height > 0 else { return }
        let time = UIAccessibility.isReduceMotionEnabled ? 0 : CGFloat(timestamp - startedAt)
        let radius = min(bounds.width, bounds.height) * 0.4 * parameters.radius
        let center = CGPoint(x: bounds.midX, y: bounds.midY)
        let cosine = cos(time * parameters.rotation)
        let sine = sin(time * parameters.rotation)
        let scale = window?.screen.scale ?? traitCollection.displayScale
        let paths = (0..<buckets.count).map { _ in CGMutablePath() }

        for dot in Self.dots {
            let rotatedX = dot.x * cosine + dot.z * sine
            let rotatedZ = -dot.x * sine + dot.z * cosine
            let rotatedY = dot.y * Self.tiltCosine - rotatedZ * Self.tiltSine
            let depth = dot.y * Self.tiltSine + rotatedZ * Self.tiltCosine
            let noise = sin(2.1 * rotatedX + 1.3 * time)
                * sin(1.7 * rotatedY - 0.9 * time)
                * sin(2.6 * rotatedZ + 0.7 * time)
                + 0.5 * sin(3.9 * rotatedX - 1.1 * time + dot.phase)
                * sin(3.1 * rotatedZ + 1.5 * time)
            let displacement = 1 + parameters.amplitude * noise + parameters.breathing * sin(time * 0.9)
            let x = center.x + rotatedX * radius * displacement
            let y = center.y - rotatedY * radius * displacement
            let depth01 = min(1, max(0, (depth + 1) * 0.5))
            let bucket = min(buckets.count - 1, Int(depth01 * CGFloat(buckets.count)))
            let side = 0.9 + 1.4 * depth01
            let pixelSide = max(1 / scale, round(side * scale) / scale)
            let pixelX = round((x - pixelSide * 0.5) * scale) / scale
            let pixelY = round((y - pixelSide * 0.5) * scale) / scale
            let rect = CGRect(x: pixelX, y: pixelY, width: pixelSide, height: pixelSide)
            paths[bucket].addRect(rect)
        }

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (index, bucket) in buckets.enumerated() {
            bucket.opacity = Self.baseBucketOpacities[index] * Float(parameters.opacity)
            bucket.path = paths[index]
        }
        CATransaction.commit()
    }

    private func updateInkColor() {
        let color = Self.ink.resolvedColor(with: traitCollection).cgColor
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for bucket in buckets {
            bucket.fillColor = color
            bucket.contentsScale = traitCollection.displayScale
        }
        CATransaction.commit()
    }
}

@MainActor
private final class DisplayLinkProxy: NSObject {
    weak var target: DotOrbView?

    @objc func step(_ link: CADisplayLink) {
        target?.displayTick(at: link.timestamp)
    }
}
