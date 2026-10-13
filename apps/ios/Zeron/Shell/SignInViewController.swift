import AuthenticationServices
import UIKit

enum Endpoints {
    static let edgeURL = URL(string: "https://cosmos-edge.cosmos-edge.workers.dev")!
    static let callbackScheme = "dove"
}

/// Sign in with Woozlit (system web auth sheet).
final class SignInViewController: UIViewController, ASWebAuthenticationPresentationContextProviding {
    private let app: AppModel
    private let status = UILabel()
    private let signIn = UIButton(type: .system)
    private var session: ASWebAuthenticationSession?

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        let mark = UIImageView(image: UIImage(systemName: "sparkle", withConfiguration: UIImage.SymbolConfiguration(pointSize: 44, weight: .light)))
        mark.tintColor = Palette.text
        let title = UILabel()
        title.text = "Dove"
        title.font = Fonts.ui(.sansSemibold, 34)
        title.textColor = Palette.text
        let tagline = UILabel()
        tagline.text = "Your coding agents, from anywhere."
        tagline.font = Fonts.ui(.sans, 17)
        tagline.textColor = Palette.secondary

        var primary = UIButton.Configuration.filled()
        primary.title = "Sign In"
        primary.baseBackgroundColor = Palette.text
        primary.baseForegroundColor = Palette.background
        primary.cornerStyle = .capsule
        primary.contentInsets = NSDirectionalEdgeInsets(top: 15, leading: 20, bottom: 15, trailing: 20)
        primary.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
            var a = a
            a.font = Fonts.ui(.sansSemibold, 17)
            a.foregroundColor = Palette.background
            return a
        }
        signIn.configuration = primary
        signIn.accessibilityIdentifier = "sign-in"
        signIn.addAction(UIAction { [weak self] _ in self?.startWoozlit() }, for: .touchUpInside)

        status.font = Fonts.ui(.sans, 14)
        status.textColor = Palette.danger
        status.numberOfLines = 0
        status.textAlignment = .center

        let top = UIStackView(arrangedSubviews: [mark, title, tagline])
        top.axis = .vertical
        top.alignment = .center
        top.spacing = 12
        let buttons = UIStackView(arrangedSubviews: [status, signIn])
        buttons.axis = .vertical
        buttons.spacing = 12
        for v in [top, buttons] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        NSLayoutConstraint.activate([
            top.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            top.centerYAnchor.constraint(equalTo: view.centerYAnchor, constant: -80),
            buttons.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            buttons.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            buttons.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24),
        ])
    }

    private func startWoozlit() {
        let state = UUID().uuidString
        let signInURL = URL(string: woozlitSignInUrl(state: state))!
        let s = ASWebAuthenticationSession(url: signInURL, callback: .customScheme(Endpoints.callbackScheme)) { [weak self] url, error in
            guard let self else { return }
            if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin { return }
            guard let url, let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
                  let callbackState = items.first(where: { $0.name == "state" })?.value,
                  callbackState == state,
                  let token = (items.first(where: { $0.name == "token" })?.value
                    ?? items.first(where: { $0.name == "code" })?.value)?.nonEmpty
            else {
                self.status.text = "Sign-in didn't complete. Try again."
                return
            }
            let userId = items.first(where: { $0.name == "userId" })?.value
            let email = items.first(where: { $0.name == "email" })?.value
            let displayName = items.first(where: { $0.name == "displayName" })?.value
            self.signIn.configuration?.showsActivityIndicator = true
            Task { @MainActor in
                do {
                    try await self.app.signInWoozlit(token: token, userId: userId, email: email, displayName: displayName)
                } catch {
                    self.status.text = error.localizedDescription
                }
                self.signIn.configuration?.showsActivityIndicator = false
            }
        }
        s.presentationContextProvider = self
        s.prefersEphemeralWebBrowserSession = false
        session = s
        s.start()
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        view.window ?? ASPresentationAnchor()
    }
}
