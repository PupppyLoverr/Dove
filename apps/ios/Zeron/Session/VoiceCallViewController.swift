import AVFoundation
import UIKit

private enum VoiceCallColors {
    static let ink = UIColor { traits in
        traits.userInterfaceStyle == .dark ? .white : .black
    }
    static let paper = UIColor { traits in
        traits.userInterfaceStyle == .dark ? .black : .white
    }

    static func inkWithAlpha(_ alpha: CGFloat) -> UIColor {
        UIColor { traits in
            (traits.userInterfaceStyle == .dark ? UIColor.white : UIColor.black)
                .withAlphaComponent(alpha)
        }
    }
}

@MainActor
final class VoiceCallViewController: UIViewController, @preconcurrency AVSpeechSynthesizerDelegate {
    private enum Phase: Equatable {
        case listening
        case sending
        case working
        case speaking
        case paused
        case unavailable(String)
    }

    private let source: SessionSource
    private let latestReply: () -> String?
    private let harness: String
    private var modelLabel: String
    private let demoPreview: Bool
    private let capture = SpeechCapture(
        mode: .voiceChat,
        silenceTimeout: 1.4,
        maximumDuration: 60,
        deactivateSessionOnStop: false,
        ducksOthers: false
    )
    private let speech = AVSpeechSynthesizer()
    private let model = UILabel()
    private let modelMark = UIImageView()
    private let modelChip = UIView()
    private let status = UILabel()
    private let caption = UILabel()
    private let orb = DotOrbView()
    private let muteButton = UIButton(type: .system)
    private let endButton = UIButton(type: .system)
    private let controls = UIStackView()
    private var listenTask: Task<Void, Never>?
    private var replyTask: Task<Void, Never>?
    private var demoTask: Task<Void, Never>?
    private var phase: Phase = .paused
    private var muted = false
    private var ended = false
    private var listening = false
    private var interrupted = false
    private var observedRunning = false
    private var sentRevision: UInt64 = 0
    private var sentRows: UInt32 = 0
    private var currentFrameRevision: UInt64 = 0
    private var currentFrameRows: UInt32 = 0
    private var currentChrome = SessionChrome()
    private var lastUtterance = ""
    private var previousIdleTimerState = false

    init(
        source: SessionSource,
        title: String,
        model: String,
        harness: String,
        latestReply: @escaping () -> String?
    ) {
        self.source = source
        self.modelLabel = model
        self.harness = harness
        self.latestReply = latestReply
        let arguments = ProcessInfo.processInfo.arguments
        demoPreview = arguments.contains("-demo") && arguments.contains("-voicePhases")
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .overFullScreen
        modalPresentationCapturesStatusBarAppearance = true
    }

    override var preferredStatusBarStyle: UIStatusBarStyle { .default }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        speech.delegate = self
        previousIdleTimerState = UIApplication.shared.isIdleTimerDisabled
        UIApplication.shared.isIdleTimerDisabled = true
        view.backgroundColor = VoiceCallColors.paper

        modelChip.translatesAutoresizingMaskIntoConstraints = false
        modelMark.contentMode = .scaleAspectFit
        modelMark.tintColor = VoiceCallColors.inkWithAlpha(0.5)
        modelMark.setContentHuggingPriority(.required, for: .horizontal)
        model.textColor = VoiceCallColors.inkWithAlpha(0.5)
        model.font = Fonts.ui(.sans, 13)
        model.numberOfLines = 1
        let modelContents = UIStackView(arrangedSubviews: [modelMark, model])
        modelContents.axis = .horizontal
        modelContents.alignment = .center
        modelContents.spacing = 7
        modelContents.translatesAutoresizingMaskIntoConstraints = false
        modelChip.addSubview(modelContents)
        NSLayoutConstraint.activate([
            modelContents.leadingAnchor.constraint(equalTo: modelChip.leadingAnchor, constant: 10),
            modelContents.trailingAnchor.constraint(equalTo: modelChip.trailingAnchor, constant: -10),
            modelContents.topAnchor.constraint(equalTo: modelChip.topAnchor, constant: 6),
            modelContents.bottomAnchor.constraint(equalTo: modelChip.bottomAnchor, constant: -6),
            modelMark.widthAnchor.constraint(equalToConstant: 14),
            modelMark.heightAnchor.constraint(equalToConstant: 14),
        ])
        updateModelLabel()

        status.textColor = VoiceCallColors.inkWithAlpha(0.55)
        status.font = Fonts.ui(.sansMedium, 14)
        status.numberOfLines = 1
        status.translatesAutoresizingMaskIntoConstraints = false

        caption.textColor = VoiceCallColors.inkWithAlpha(0.9)
        caption.font = Fonts.ui(.sans, 17)
        caption.textAlignment = .center
        caption.numberOfLines = 4
        caption.lineBreakMode = .byTruncatingTail
        caption.adjustsFontForContentSizeCategory = true
        caption.translatesAutoresizingMaskIntoConstraints = false
        caption.setContentCompressionResistancePriority(.defaultLow, for: .vertical)

        orb.accessibilityLabel = "Voice activity"
        orb.translatesAutoresizingMaskIntoConstraints = false
        orb.isUserInteractionEnabled = true
        let centerTap = UITapGestureRecognizer(target: self, action: #selector(orbTapped))
        orb.addGestureRecognizer(centerTap)

        configureButton(
            muteButton,
            symbol: "mic",
            background: VoiceCallColors.inkWithAlpha(0.08),
            tint: VoiceCallColors.ink
        )
        configureButton(
            endButton,
            symbol: "xmark",
            background: VoiceCallColors.ink,
            tint: VoiceCallColors.paper,
            weight: .semibold
        )
        muteButton.accessibilityIdentifier = "voice-mute"
        muteButton.accessibilityLabel = "Mute"
        muteButton.addAction(UIAction { [weak self] _ in self?.toggleMute() }, for: .touchUpInside)
        endButton.accessibilityIdentifier = "voice-end"
        endButton.accessibilityLabel = "End voice mode"
        endButton.addAction(UIAction { [weak self] _ in self?.endCall() }, for: .touchUpInside)
        controls.axis = .horizontal
        controls.alignment = .center
        controls.spacing = 32
        controls.addArrangedSubview(muteButton)
        controls.addArrangedSubview(endButton)
        controls.translatesAutoresizingMaskIntoConstraints = false

        [modelChip, status, orb, caption, controls].forEach(view.addSubview)

        NSLayoutConstraint.activate([
            modelChip.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            modelChip.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            status.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            status.topAnchor.constraint(equalTo: orb.bottomAnchor, constant: 24),
            orb.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            orb.centerYAnchor.constraint(equalTo: view.centerYAnchor, constant: -56),
            orb.widthAnchor.constraint(equalToConstant: 232),
            orb.heightAnchor.constraint(equalTo: orb.widthAnchor),
            caption.topAnchor.constraint(equalTo: status.bottomAnchor, constant: 10),
            caption.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 28),
            caption.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -28),
            caption.bottomAnchor.constraint(lessThanOrEqualTo: controls.topAnchor, constant: -22),
            controls.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            controls.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24),
            muteButton.widthAnchor.constraint(equalToConstant: 56),
            muteButton.heightAnchor.constraint(equalToConstant: 56),
            endButton.widthAnchor.constraint(equalToConstant: 56),
            endButton.heightAnchor.constraint(equalToConstant: 56),
        ])

        setPhase(.paused)
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(interruption(_:)),
            name: AVAudioSession.interruptionNotification,
            object: nil
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(backgrounded),
            name: UIApplication.didEnterBackgroundNotification,
            object: nil
        )
        if demoPreview {
            startDemoPreview()
        } else {
            startListening()
        }
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if !ended { finishWithoutDismiss() }
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        listenTask?.cancel()
        replyTask?.cancel()
        demoTask?.cancel()
    }

    func sessionDidUpdate(chrome: SessionChrome, frame: LayoutFrame) {
        currentChrome = chrome
        currentFrameRevision = frame.revision()
        currentFrameRows = frame.rowCount()
        if let model = chrome.chips.first(where: { $0.id == "model" })?.title {
            modelLabel = model
            updateModelLabel()
        }
        guard !ended, !demoPreview else { return }
        if chrome.running {
            observedRunning = true
            if !listening, !speech.isSpeaking { setPhase(.working) }
        } else if observedRunning || frameChangedSinceSend {
            scheduleReplyCheck()
        }
        if case .failed = chrome.banner, phase == .sending || phase == .working {
            speak("Something went wrong — check the chat.")
        }
    }

    @objc private func orbTapped() {
        guard phase == .speaking else { return }
        if demoPreview {
            demoTask?.cancel()
            demoTask = nil
            setCaption(nil)
            startDemoPreview()
            return
        }
        guard speech.isSpeaking else { return }
        speech.stopSpeaking(at: .immediate)
        setCaption(nil)
        startListening()
    }

    private func startListening() {
        guard !ended, !muted, !interrupted, !listening, !speech.isSpeaking else { return }
        replyTask?.cancel()
        replyTask = nil
        lastUtterance = ""
        setCaption(nil)
        setPhase(.listening)
        listenTask?.cancel()
        listenTask = Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                try await self.capture.start(
                    onText: { [weak self] text in
                        guard let self else { return }
                        self.lastUtterance = text
                        self.setCaption(text)
                    },
                    onEnd: { [weak self] error in self?.listeningEnded(error) },
                    onLevel: { [weak self] level in self?.updateOrb(level: level) }
                )
                guard !self.ended, !self.muted else { return }
                self.listening = true
            } catch {
                guard !Task.isCancelled, !self.ended else { return }
                if (error as? DictationError) == .permissionDenied {
                    self.setPhase(.unavailable("Microphone access is off"))
                    self.presentSpeechSettingsAlert()
                } else {
                    self.setPhase(.unavailable((error as? DictationError)?.message ?? "Voice isn't available right now"))
                }
            }
        }
    }

    private func listeningEnded(_ error: Error?) {
        listening = false
        guard !ended, !muted, !interrupted else { return }
        if error != nil, lastUtterance.isEmpty {
            setPhase(.paused)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { [weak self] in self?.startListening() }
            return
        }
        let text = lastUtterance.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else {
            startListening()
            return
        }
        send(text: text)
    }

    private func send(text: String) {
        sentRevision = currentFrameRevision
        sentRows = currentFrameRows
        observedRunning = false
        lastUtterance = ""
        setCaption(text)
        setPhase(.sending)
        let mode: DeliveryMode = currentChrome.running && currentChrome.canSteer ? .steer : .queue
        guard source.send(text: text, images: [], mode: mode) else {
            setPhase(.paused)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in self?.startListening() }
            return
        }
    }

    private var frameChangedSinceSend: Bool {
        currentFrameRevision != sentRevision || currentFrameRows != sentRows
    }

    private func scheduleReplyCheck() {
        guard replyTask == nil, !ended else { return }
        replyTask = Task { @MainActor [weak self] in
            guard let self else { return }
            for _ in 0..<24 {
                try? await Task.sleep(nanoseconds: 200_000_000)
                guard !Task.isCancelled, !self.ended else { return }
                guard !self.currentChrome.running else { continue }
                if self.frameChangedSinceSend, let reply = self.latestReply(), !reply.isEmpty {
                    self.replyTask = nil
                    self.speak(reply)
                    return
                }
            }
            self.replyTask = nil
            self.startListening()
        }
    }

    private func speak(_ text: String) {
        guard !ended else { return }
        let cleaned = SpeechText.clean(text)
        guard !cleaned.isEmpty else { startListening(); return }
        capture.stop()
        listening = false
        setCaption(cleaned)
        setPhase(.speaking)
        let utterance = AVSpeechUtterance(string: cleaned)
        utterance.rate = 0.5
        utterance.voice = preferredVoice()
        speech.speak(utterance)
    }

    private func preferredVoice() -> AVSpeechSynthesisVoice? {
        let language = Locale.current.language.languageCode?.identifier ?? "en"
        let voices = AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix(language) }
        return voices.max { quality($0.quality) < quality($1.quality) }
            ?? AVSpeechSynthesisVoice(language: Locale.current.identifier)
            ?? AVSpeechSynthesisVoice(language: "en-US")
    }

    private func quality(_ quality: AVSpeechSynthesisVoiceQuality) -> Int {
        switch quality {
        case .premium: 3
        case .enhanced: 2
        default: 1
        }
    }

    func speechSynthesizer(
        _ synthesizer: AVSpeechSynthesizer,
        willSpeakRangeOfSpeechString characterRange: NSRange,
        utterance: AVSpeechUtterance
    ) {
        orb.pulse()
    }

    func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        guard !ended else { return }
        setCaption(nil)
        startListening()
    }

    private func updateOrb(level: Float) {
        orb.setLevel(min(1, CGFloat(level) * 8))
    }

    private func toggleMute() {
        muted.toggle()
        updateMuteButton()
        if muted {
            lastUtterance = ""
            capture.stop()
            speech.stopSpeaking(at: .immediate)
            listening = false
            setCaption(nil)
            setPhase(.paused)
            if demoPreview {
                demoTask?.cancel()
                demoTask = nil
            }
        } else if demoPreview {
            startDemoPreview()
        } else {
            startListening()
        }
    }

    private func setPhase(_ phase: Phase) {
        self.phase = phase
        let statusText: String
        switch phase {
        case .listening:
            statusText = "Listening"
            orb.mode = .listening
            orb.accessibilityLabel = "Voice activity"
        case .sending:
            statusText = "Sending"
            orb.mode = .thinking
            orb.accessibilityLabel = "Voice activity"
        case .working:
            statusText = "Thinking"
            orb.mode = .thinking
            orb.accessibilityLabel = "Voice activity"
        case .speaking:
            statusText = "Speaking · Tap to interrupt"
            orb.mode = .speaking
            orb.accessibilityLabel = "Barge in"
        case .paused:
            statusText = "Paused"
            orb.mode = .paused
            orb.accessibilityLabel = "Voice activity"
        case let .unavailable(message):
            statusText = message
            orb.mode = .paused
            orb.accessibilityLabel = "Voice activity"
        }
        status.text = statusText
        orb.accessibilityValue = statusText
        orb.accessibilityHint = phase == .speaking ? "Double-tap to interrupt" : nil
    }

    private func setCaption(_ text: String?) {
        guard caption.text != text else { return }
        guard !UIAccessibility.isReduceMotionEnabled, viewIfLoaded?.window != nil else {
            caption.text = text
            return
        }
        UIView.transition(
            with: caption,
            duration: 0.18,
            options: [.transitionCrossDissolve, .beginFromCurrentState, .allowUserInteraction]
        ) {
            self.caption.text = text
        }
    }

    private func updateModelLabel() {
        model.text = modelLabel
        modelMark.image = BrandMarks.image(for: harness, side: 14)
        modelMark.isHidden = modelMark.image == nil
    }

    private func configureButton(
        _ button: UIButton,
        symbol: String,
        background: UIColor,
        tint: UIColor,
        weight: UIImage.SymbolWeight = .medium
    ) {
        var configuration = UIButton.Configuration.plain()
        configuration.image = UIImage(
            systemName: symbol,
            withConfiguration: UIImage.SymbolConfiguration(pointSize: 18, weight: weight)
        )
        configuration.baseForegroundColor = tint
        configuration.contentInsets = .zero
        configuration.cornerStyle = .capsule
        button.configuration = configuration
        button.backgroundColor = background
        button.layer.cornerRadius = 28
        button.clipsToBounds = true
        button.translatesAutoresizingMaskIntoConstraints = false
    }

    private func updateMuteButton() {
        var configuration = muteButton.configuration ?? UIButton.Configuration.plain()
        configuration.image = UIImage(
            systemName: muted ? "mic.slash" : "mic",
            withConfiguration: UIImage.SymbolConfiguration(pointSize: 18, weight: .medium)
        )
        configuration.baseForegroundColor = muted ? VoiceCallColors.paper : VoiceCallColors.ink
        muteButton.configuration = configuration
        muteButton.backgroundColor = muted ? VoiceCallColors.ink : VoiceCallColors.inkWithAlpha(0.08)
        muteButton.accessibilityLabel = muted ? "Unmute" : "Mute"
    }

    private func startDemoPreview() {
        guard !ended, !muted else { return }
        demoTask?.cancel()
        demoTask = Task { @MainActor [weak self] in
            guard let self else { return }
            while !Task.isCancelled, !self.ended, !self.muted {
                self.listening = true
                self.setCaption(nil)
                self.setPhase(.listening)
                let listeningUntil = CACurrentMediaTime() + 3
                var sample = 0
                while CACurrentMediaTime() < listeningUntil, !Task.isCancelled, !self.ended, !self.muted {
                    let t = CGFloat(sample) * 0.31
                    let noisyLevel = abs(sin(t)) * (0.35 + 0.65 * abs(sin(t * 2.37)))
                    self.orb.setLevel(noisyLevel)
                    sample += 1
                    try? await Task.sleep(nanoseconds: 80_000_000)
                }
                guard !Task.isCancelled, !self.ended, !self.muted else { return }
                self.orb.setLevel(0)
                self.listening = false
                self.setCaption("Can you summarize the latest changes?")
                self.setPhase(.sending)
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard !Task.isCancelled, !self.ended, !self.muted else { return }
                self.setPhase(.working)
                try? await Task.sleep(nanoseconds: 3_000_000_000)
                guard !Task.isCancelled, !self.ended, !self.muted else { return }
                self.setCaption(self.latestReply() ?? "The latest changes are ready to review.")
                self.setPhase(.speaking)
                for _ in 0..<16 {
                    guard !Task.isCancelled, !self.ended, !self.muted else { return }
                    self.orb.pulse()
                    try? await Task.sleep(nanoseconds: 250_000_000)
                }
            }
        }
    }

    private func endCall() {
        guard !ended else { return }
        finishWithoutDismiss()
        dismiss(animated: true)
    }

    private func finishWithoutDismiss() {
        guard !ended else { return }
        ended = true
        listenTask?.cancel()
        replyTask?.cancel()
        demoTask?.cancel()
        capture.stop()
        speech.stopSpeaking(at: .immediate)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        UIApplication.shared.isIdleTimerDisabled = previousIdleTimerState
        NotificationCenter.default.removeObserver(self)
    }

    @objc private func interruption(_ note: Notification) {
        guard !ended else { return }
        guard let typeValue = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: typeValue)
        else { return }
        if type == .began {
            interrupted = true
            lastUtterance = ""
            capture.stop()
            speech.stopSpeaking(at: .immediate)
            listening = false
            setCaption(nil)
            setPhase(.paused)
        } else if type == .ended, !muted {
            interrupted = false
            startListening()
        }
    }

    @objc private func backgrounded() {
        endCall()
    }

    private func presentSpeechSettingsAlert() {
        let alert = UIAlertController(
            title: "Microphone or Speech Recognition access is off",
            message: nil,
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "Open Settings", style: .default) { [weak self] _ in
            self?.endCall()
            guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
            UIApplication.shared.open(url)
        })
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { [weak self] _ in self?.endCall() })
        present(alert, animated: true)
    }
}
