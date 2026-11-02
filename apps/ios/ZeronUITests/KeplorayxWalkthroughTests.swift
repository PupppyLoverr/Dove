import XCTest

/// Keplorayx's way in and the main screens, photographed in order. The
/// sign-in test stops at Woozlit's page and never enters credentials.
final class KeplorayxWalkthroughTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = true
    }

    private func launch(_ args: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = args
        app.launch()
        return app
    }

    private func snapshot(_ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    func test1Intro() {
        let app = launch(["-signedout", "-onboardingSeen", "NO"])
        let next = app.buttons["sign-in"]
        XCTAssertTrue(next.waitForExistence(timeout: 15))
        sleep(2)
        snapshot("intro-1-keplorayx")
        XCTAssertTrue(app.buttons["skip-intro"].exists, "skip on the first page")
        next.tap()
        sleep(2)
        XCTAssertTrue(app.staticTexts["Every agent, one place"].exists)
        snapshot("intro-2-every-agent")
        next.tap()
        sleep(2)
        XCTAssertTrue(app.staticTexts["Steer from anywhere"].exists)
        snapshot("intro-3-steer")
        next.tap()
        sleep(2)
        XCTAssertTrue(app.staticTexts["Sign in to sync"].exists)
        snapshot("intro-4-sign-in")
        app.buttons["try-demo"].tap()
        XCTAssertTrue(app.staticTexts["Sessions"].waitForExistence(timeout: 15), "look around opens the sessions")
        sleep(2)
        snapshot("intro-5-look-around")
    }

    func test2SignInOpensWoozlit() {
        let app = launch(["-signedout", "-onboardingSeen", "YES"])
        let signIn = app.buttons["sign-in"]
        XCTAssertTrue(signIn.waitForExistence(timeout: 15))
        sleep(1)
        signIn.tap()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.buttons["Continue"]
        if allow.waitForExistence(timeout: 8) {
            snapshot("signin-1-system-prompt")
            allow.tap()
        }
        sleep(10)
        snapshot("signin-2-woozlit")
        // Back out without signing in: no error, and the page still works.
        let cancel = app.buttons["Cancel"]
        if cancel.exists { cancel.tap() }
        XCTAssertTrue(signIn.waitForExistence(timeout: 10), "back on the sign-in page")
        sleep(1)
        XCTAssertTrue(signIn.isEnabled)
        let status = app.staticTexts["sign-in-status"]
        XCTAssertTrue(!status.exists || status.label.isEmpty, "cancelling says nothing")
        snapshot("signin-3-cancelled")
    }

    func test3Screens() {
        let app = launch(["-demo"])
        XCTAssertTrue(app.staticTexts["Sessions"].waitForExistence(timeout: 15))
        sleep(2)
        snapshot("sessions-1-list")
        app.collectionViews.firstMatch.swipeUp()
        sleep(1)
        snapshot("sessions-2-scrolled")

        let chat = launch(["-demo", "-route", "chat:chat-veil"])
        XCTAssertTrue(chat.textViews["composer-input"].waitForExistence(timeout: 15))
        sleep(3)
        snapshot("chat-1-transcript")

        let new = launch(["-demo", "-route", "new"])
        sleep(4)
        snapshot("new-session-1")
        _ = new

        let settings = launch(["-demo", "-route", "more"])
        sleep(4)
        snapshot("settings-1")
        settings.swipeUp()
        sleep(1)
        snapshot("settings-2-scrolled")
    }
}
