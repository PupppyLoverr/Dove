import AVFoundation
import Speech

enum DictationError: Error, Equatable {
    case permissionDenied
    case recognizerUnavailable
    case audioInputUnavailable

    var message: String {
        switch self {
        case .permissionDenied: "Microphone or Speech Recognition access is off"
        case .recognizerUnavailable: "Speech recognition isn't available right now"
        case .audioInputUnavailable: "No microphone available"
        }
    }
}

@MainActor
final class SpeechCapture {
    private let mode: AVAudioSession.Mode
    private let silenceTimeout: TimeInterval
    private let maximumDuration: TimeInterval
    private let deactivateSessionOnStop: Bool
    private let ducksOthers: Bool
    private let engine = AVAudioEngine()
    private var recognizer: SFSpeechRecognizer?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var recognitionTask: SFSpeechRecognitionTask?
    private var silenceTask: Task<Void, Never>?
    private var durationTask: Task<Void, Never>?
    private var onText: ((String) -> Void)?
    private var onEnd: ((Error?) -> Void)?
    private var onLevel: ((Float) -> Void)?
    private var isRunning = false
    private var tapInstalled = false
    private var audioSessionActive = false
    private var heardSpeech = false

    init(
        mode: AVAudioSession.Mode,
        silenceTimeout: TimeInterval,
        maximumDuration: TimeInterval,
        deactivateSessionOnStop: Bool,
        ducksOthers: Bool
    ) {
        self.mode = mode
        self.silenceTimeout = silenceTimeout
        self.maximumDuration = maximumDuration
        self.deactivateSessionOnStop = deactivateSessionOnStop
        self.ducksOthers = ducksOthers
    }

    func start(
        onText: @escaping (String) -> Void,
        onEnd: @escaping (Error?) -> Void,
        onLevel: ((Float) -> Void)? = nil
    ) async throws {
        if isRunning { stop() }
        try await Self.requestPermissions()
        try Task.checkCancellation()

        guard let recognizer = SFSpeechRecognizer(locale: .current)
            ?? SFSpeechRecognizer(locale: Locale(identifier: "en-US"))
        else {
            throw DictationError.recognizerUnavailable
        }

        self.recognizer = recognizer
        self.onText = onText
        self.onEnd = onEnd
        self.onLevel = onLevel
        heardSpeech = false

        let session = AVAudioSession.sharedInstance()
        var options: AVAudioSession.CategoryOptions = [.defaultToSpeaker, .allowBluetoothHFP]
        if ducksOthers { options.insert(.duckOthers) }
        do {
            try session.setCategory(.playAndRecord, mode: mode, options: options)
            try session.setActive(true)
            audioSessionActive = true

            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            guard format.channelCount > 0, format.sampleRate > 0 else { throw DictationError.audioInputUnavailable }
            let request = SFSpeechAudioBufferRecognitionRequest()
            request.shouldReportPartialResults = true
            request.requiresOnDeviceRecognition = recognizer.supportsOnDeviceRecognition
            request.addsPunctuation = true
            self.request = request

            input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self, weak request] buffer, _ in
                guard let request else { return }
                request.append(buffer)
                guard let channel = buffer.floatChannelData?.pointee, buffer.frameLength > 0 else { return }
                let count = Int(buffer.frameLength)
                var energy: Float = 0
                for index in 0..<count { energy += channel[index] * channel[index] }
                let level = sqrt(energy / Float(count))
                DispatchQueue.main.async { [weak self] in self?.onLevel?(level) }
            }
            tapInstalled = true
            engine.prepare()
            try engine.start()
            isRunning = true
            recognitionTask = recognizer.recognitionTask(with: request) { [weak self] result, error in
                Task { @MainActor [weak self] in
                    guard let self, self.isRunning else { return }
                    if let error {
                        self.finish(error)
                        return
                    }
                    guard let result else { return }
                    let text = result.bestTranscription.formattedString
                    if !text.isEmpty {
                        self.heardSpeech = true
                        self.onText?(text)
                        self.resetSilenceTimer()
                    }
                    if result.isFinal { self.finish(nil) }
                }
            }
            durationTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(maximumDuration * 1_000_000_000))
                guard let self, !Task.isCancelled, self.isRunning else { return }
                self.finish(nil)
            }
        } catch {
            teardown()
            throw error
        }
    }

    func stop() {
        finish(nil)
    }

    private func resetSilenceTimer() {
        guard heardSpeech else { return }
        silenceTask?.cancel()
        silenceTask = Task { [weak self] in
            guard let self else { return }
            try? await Task.sleep(nanoseconds: UInt64(self.silenceTimeout * 1_000_000_000))
            guard !Task.isCancelled, self.isRunning else { return }
            self.finish(nil)
        }
    }

    private func finish(_ error: Error?) {
        guard isRunning else { return }
        let callback = onEnd
        teardown()
        callback?(error)
    }

    private func teardown() {
        silenceTask?.cancel()
        silenceTask = nil
        durationTask?.cancel()
        durationTask = nil
        if engine.isRunning { engine.stop() }
        if tapInstalled {
            engine.inputNode.removeTap(onBus: 0)
            tapInstalled = false
        }
        request?.endAudio()
        request = nil
        recognitionTask?.cancel()
        recognitionTask = nil
        recognizer = nil
        onText = nil
        onEnd = nil
        onLevel = nil
        isRunning = false
        if audioSessionActive && deactivateSessionOnStop {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
        audioSessionActive = false
    }

    private static func requestPermissions() async throws {
        let speechStatus: SFSpeechRecognizerAuthorizationStatus = await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { status in
                continuation.resume(returning: status)
            }
        }
        guard speechStatus == .authorized else { throw DictationError.permissionDenied }

        let microphoneAllowed: Bool = await withCheckedContinuation { continuation in
            AVAudioApplication.requestRecordPermission { allowed in
                continuation.resume(returning: allowed)
            }
        }
        guard microphoneAllowed else { throw DictationError.permissionDenied }
    }
}

@MainActor
final class Dictation {
    private let capture = SpeechCapture(
        mode: .measurement,
        silenceTimeout: 2,
        maximumDuration: 60,
        deactivateSessionOnStop: true,
        ducksOthers: true
    )

    func start(onText: @escaping (String) -> Void, onEnd: @escaping (Error?) -> Void) async throws {
        try await capture.start(onText: onText, onEnd: onEnd)
    }

    func stop() {
        capture.stop()
    }
}
