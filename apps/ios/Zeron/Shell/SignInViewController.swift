import AuthenticationServices
import UIKit

enum Endpoints {
    /// Keplorayx's edge: sessions sync through it, signed in with Woozlit.
    static let edgeURL = URL(string: "https://cosmos-edge.cosmos-edge.workers.dev")!
    static let callbackScheme = "dove"

    #if DEBUG
    /// A launch argument that sticks: `-edge <url>` points the app at another
    /// edge (a local `wrangler dev`) and is remembered, so reopening it from
    /// the home screen stays there. `-edge prod` goes back.
    static func debugOverride(_ flag: String, key: String) -> String? {
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: flag), i + 1 < args.count {
            let value = args[i + 1]
            if value == "prod" {
                UserDefaults.standard.removeObject(forKey: key)
                return nil
            }
            UserDefaults.standard.set(value, forKey: key)
            return value
        }
        return UserDefaults.standard.string(forKey: key)
    }
    #endif
}

/// The way in: a flight through space with a few pages about the app (the
/// first time only), ending at sign-in with Woozlit in the system web sheet.
final class SignInViewController: UIViewController {
    /// Set once the intro's pages have been seen; Settings can clear it.
    static let seenKey = "onboardingSeen"

    private struct Page {
        let title: String
        let body: String
    }

    private static let pages = [
        Page(title: "Keplorayx", body: "Your coding agents, wherever you are."),
        Page(title: "Every agent, one place", body: "Claude Code, Codex, Gemini and more, all running on your own computers."),
        Page(title: "Steer from anywhere", body: "Follow a session live, answer its questions and hand it new work from your phone."),
    ]

    private let app: AppModel
    private let space = SpaceField()
    private let mark = UIImageView(image: K22Icon.image("k22-mark", pointSize: 56))
    private let titleLabel = UILabel()
    private let bodyLabel = UILabel()
    private let copy = UIStackView()
    private let status = UILabel()
    private let primary = UIButton(type: .system)
    private let secondary = UIButton(type: .system)
    private let skip = UIButton(type: .system)
    private let fineprint = UILabel()
    private let dots = UIStackView()
    private var dotRow: UIView?
    private let signIn = WoozlitWebSignIn()
    /// `pages.count` is the sign-in page.
    private var index = 0

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        // Space is dark whatever the phone's appearance.
        overrideUserInterfaceStyle = .dark
        view.backgroundColor = .black
        index = UserDefaults.standard.bool(forKey: Self.seenKey) ? Self.pages.count : 0

        space.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(space)

        mark.tintColor = .white
        mark.contentMode = .scaleAspectFit
        titleLabel.font = Fonts.ui(.sansSemibold, 32)
        titleLabel.textColor = .white
        titleLabel.textAlignment = .center
        titleLabel.numberOfLines = 0
        bodyLabel.font = Fonts.ui(.sans, 17)
        bodyLabel.textColor = UIColor(white: 1, alpha: 0.7)
        bodyLabel.textAlignment = .center
        bodyLabel.numberOfLines = 0
        copy.axis = .vertical
        copy.alignment = .center
        copy.spacing = 12
        for v in [mark, titleLabel, bodyLabel] as [UIView] { copy.addArrangedSubview(v) }
        copy.setCustomSpacing(24, after: mark)

        status.font = Fonts.ui(.sans, 14)
        status.textColor = Palette.danger
        status.numberOfLines = 0
        status.textAlignment = .center
        status.accessibilityIdentifier = "sign-in-status"

        primary.accessibilityIdentifier = "sign-in"
        primary.addAction(UIAction { [weak self] _ in self?.primaryTapped() }, for: .touchUpInside)

        var quiet = UIButton.Configuration.plain()
        quiet.baseForegroundColor = UIColor(white: 1, alpha: 0.7)
        quiet.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
            var a = a
            a.font = Fonts.ui(.sansMedium, 15)
            return a
        }
        secondary.configuration = quiet
        secondary.configuration?.title = "Look around first"
        secondary.accessibilityIdentifier = "try-demo"
        secondary.addAction(UIAction { [weak self] _ in self?.app.startDemo() }, for: .touchUpInside)
        skip.configuration = quiet
        skip.configuration?.title = "Skip"
        skip.accessibilityIdentifier = "skip-intro"
        skip.addAction(UIAction { [weak self] _ in self?.go(to: Self.pages.count) }, for: .touchUpInside)

        fineprint.text = "The same account as Keplorayx on your computer. Your code stays on your computers."
        fineprint.font = Fonts.ui(.sans, 12)
        fineprint.textColor = UIColor(white: 1, alpha: 0.45)
        fineprint.textAlignment = .center
        fineprint.numberOfLines = 0

        dots.axis = .horizontal
        dots.spacing = 8
        for _ in 0...Self.pages.count {
            let dot = UIView()
            dot.layer.cornerRadius = 3
            dot.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([dot.widthAnchor.constraint(equalToConstant: 6), dot.heightAnchor.constraint(equalToConstant: 6)])
            dots.addArrangedSubview(dot)
        }
        // Centred in a row of their own: a filling stack would stretch the last.
        dots.translatesAutoresizingMaskIntoConstraints = false
        let dotRow = UIView()
        dotRow.addSubview(dots)
        NSLayoutConstraint.activate([
            dots.centerXAnchor.constraint(equalTo: dotRow.centerXAnchor),
            dots.topAnchor.constraint(equalTo: dotRow.topAnchor),
            dots.bottomAnchor.constraint(equalTo: dotRow.bottomAnchor),
        ])
        self.dotRow = dotRow

        let bottom = UIStackView(arrangedSubviews: [status, dotRow, primary, secondary, fineprint])
        bottom.axis = .vertical
        bottom.alignment = .fill
        bottom.spacing = 14
        bottom.setCustomSpacing(4, after: primary)
        bottom.setCustomSpacing(8, after: secondary)
        for v in [copy, bottom, skip] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        NSLayoutConstraint.activate([
            space.topAnchor.constraint(equalTo: view.topAnchor),
            space.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            space.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            space.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            copy.centerYAnchor.constraint(equalTo: view.centerYAnchor, constant: -70),
            copy.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 32),
            copy.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -32),
            bottom.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            bottom.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            bottom.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -16),
            skip.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 4),
            skip.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
        ])
        show(index)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        space.warp(1.6)
    }

    // MARK: Pages

    private var onSignIn: Bool { index >= Self.pages.count }

    private func show(_ i: Int) {
        index = i
        if onSignIn {
            titleLabel.text = "Sign in to sync"
            bodyLabel.text = "See your sessions here and drive them from your phone."
        } else {
            titleLabel.text = Self.pages[i].title
            bodyLabel.text = Self.pages[i].body
        }
        mark.isHidden = i != 0 && !onSignIn
        skip.isHidden = onSignIn
        secondary.isHidden = !onSignIn
        fineprint.isHidden = !onSignIn
        dotRow?.isHidden = UserDefaults.standard.bool(forKey: Self.seenKey)
        for (k, dot) in dots.arrangedSubviews.enumerated() {
            dot.backgroundColor = UIColor(white: 1, alpha: k == i ? 0.9 : 0.25)
        }
        setPrimary(busy: false)
    }

    private func setPrimary(busy: Bool) {
        var config = UIButton.Configuration.filled()
        config.title = busy ? "Signing in…" : onSignIn ? "Sign In" : "Continue"
        config.showsActivityIndicator = busy
        config.baseBackgroundColor = .white
        config.baseForegroundColor = .black
        config.cornerStyle = .capsule
        config.contentInsets = NSDirectionalEdgeInsets(top: 15, leading: 20, bottom: 15, trailing: 20)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
            var a = a
            a.font = Fonts.ui(.sansSemibold, 17)
            a.foregroundColor = .black
            return a
        }
        primary.configuration = config
        primary.isEnabled = !busy
        secondary.isEnabled = !busy
    }

    /// Fly on: the copy rushes past the viewer, the stars streak, and the next
    /// page comes up from the distance.
    private func go(to next: Int) {
        guard next != index else { return }
        if next >= Self.pages.count { UserDefaults.standard.set(true, forKey: Self.seenKey) }
        status.text = nil
        guard !UIAccessibility.isReduceMotionEnabled else { return show(next) }
        UISelectionFeedbackGenerator().selectionChanged()
        space.warp(1.5)
        UIView.animate(withDuration: 0.28, delay: 0, options: [.curveEaseIn]) {
            self.copy.transform = CGAffineTransform(scaleX: 2.4, y: 2.4)
            self.copy.alpha = 0
        } completion: { _ in
            self.show(next)
            self.copy.transform = CGAffineTransform(scaleX: 0.55, y: 0.55)
            UIView.animate(withDuration: 0.6, delay: 0.05, usingSpringWithDamping: 0.9, initialSpringVelocity: 0, options: []) {
                self.copy.transform = .identity
                self.copy.alpha = 1
            }
        }
    }

    private func primaryTapped() {
        guard onSignIn else { return go(to: index + 1) }
        status.text = nil
        Task { @MainActor in
            let error = await signIn.run(app: app, from: self) { [weak self] in
                // Back from the web sheet with a token: show the work.
                self?.setPrimary(busy: true)
                self?.space.warp(1.2)
            }
            setPrimary(busy: false)
            if let error {
                status.text = error
                UINotificationFeedbackGenerator().notificationOccurred(.error)
            }
        }
    }
}

/// Sign in with Woozlit in the system web sheet; its callback
/// (`dove://auth/callback?state=…&token=…`) carries a Firebase token that
/// the core turns into a session on Keplorayx's edge.
@MainActor
final class WoozlitWebSignIn: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?
    private weak var host: UIViewController?

    /// Signs in; returns why it didn't (nil on success or cancel).
    /// `exchanging` runs once the sheet has handed back a token.
    func run(app: AppModel, from host: UIViewController, exchanging: (() -> Void)? = nil) async -> String? {
        self.host = host
        let state = UUID().uuidString
        guard let signInURL = URL(string: woozlitSignInUrl(state: state)) else {
            return "Couldn't open sign-in. Try again."
        }
        let url: URL? = await withCheckedContinuation { cont in
            let s = ASWebAuthenticationSession(url: signInURL, callback: .customScheme(Endpoints.callbackScheme)) { url, error in
                if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
                    return cont.resume(returning: nil)
                }
                cont.resume(returning: url ?? URL(string: "dove://auth/callback?failed=1"))
            }
            s.presentationContextProvider = self
            s.prefersEphemeralWebBrowserSession = false
            session = s
            if !s.start() { cont.resume(returning: nil) }
        }
        session = nil
        guard let url else { return nil }
        guard let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
              items.first(where: { $0.name == "state" })?.value == state,
              let token = (items.first(where: { $0.name == "token" })?.value
                ?? items.first(where: { $0.name == "code" })?.value)?.nonEmpty
        else { return "Sign-in didn't complete. Try again." }
        let value = { (name: String) in items.first(where: { $0.name == name })?.value }
        exchanging?()
        do {
            try await app.signInWoozlit(token: token, userId: value("userId"), email: value("email"), displayName: value("displayName"))
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        host?.view.window ?? ASPresentationAnchor()
    }
}

/// Zeron's onboarding signs in through this name.
typealias WorkOSSignIn = WoozlitWebSignIn
