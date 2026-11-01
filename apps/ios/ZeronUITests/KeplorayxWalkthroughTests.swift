import XCTest

/// Every screen a person can reach without an account, photographed in order.
/// The sign-in test stops at the sign-in page and never enters credentials.
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

    private func reveal(_ element: XCUIElement, in app: XCUIApplication) {
        let limit = app.windows.firstMatch.frame.height * 0.75
        for _ in 0..<8 where !element.exists || !element.isHittable || element.frame.maxY > limit {
            app.collectionViews.firstMatch.swipeUp(velocity: .slow)
        }
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

    func test2SignInOpensPage() {
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
        snapshot("signin-2-page")
        // Back out without signing in: the app reports nothing and stays usable.
        let cancel = app.buttons["Cancel"]
        if cancel.exists { cancel.tap() }
        XCTAssertTrue(signIn.waitForExistence(timeout: 10), "back on the sign-in page")
        sleep(1)
        snapshot("signin-3-cancelled")
    }

    func test3Sessions() {
        let app = launch(["-demo"])
        XCTAssertTrue(app.staticTexts["Sessions"].waitForExistence(timeout: 15))
        sleep(2)
        snapshot("sessions-1-list")
        app.collectionViews.firstMatch.swipeUp()
        sleep(1)
        snapshot("sessions-2-scrolled")
        app.collectionViews.firstMatch.swipeDown()
        sleep(1)
        app.tabBars.buttons["Search"].tap()
        sleep(2)
        snapshot("search-1-empty")
        app.typeText("deploy")
        sleep(2)
        snapshot("search-2-results")
        app.buttons["Close"].firstMatch.tap()
        sleep(1)
        let accessory = app.buttons["new-session"]
        XCTAssertTrue(accessory.waitForExistence(timeout: 5))
        accessory.tap()
        sleep(2)
        snapshot("new-session-1")
        for chip in ["project", "model", "effort"] {
            let button = app.buttons["composer-chip-\(chip)"]
            guard button.waitForExistence(timeout: 3) else { continue }
            button.tap()
            sleep(2)
            snapshot("new-session-chip-\(chip)")
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)).tap()
            sleep(1)
        }
    }

    func test4Session() {
        let app = launch(["-demo", "-route", "chat:chat-veil"])
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 15))
        sleep(2)
        snapshot("chat-1-transcript")
        let expand = app.buttons["Expand"].firstMatch
        if expand.exists, expand.isHittable {
            expand.tap()
            sleep(1)
            snapshot("chat-2-tools")
        }
        input.tap()
        input.typeText("Summarize this session.")
        sleep(1)
        snapshot("chat-3-typing")
        app.buttons["composer-send"].tap()
        sleep(1)
        snapshot("chat-4-sent")
        XCTAssertTrue(app.buttons["Send message"].waitForExistence(timeout: 60), "the turn settles")
        sleep(1)
        snapshot("chat-5-reply")
        let attach = app.buttons["composer-attach"]
        if attach.exists {
            attach.tap()
            sleep(1)
            snapshot("chat-6-attach-menu")
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)).tap()
            sleep(1)
        }
        let voice = app.buttons["composer-voice"]
        if voice.exists, voice.isHittable {
            voice.tap()
            let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
            let allow = springboard.buttons["Allow"]
            if allow.waitForExistence(timeout: 4) { allow.tap() }
            sleep(3)
            snapshot("chat-7-voice")
            let end = app.buttons["voice-end"]
            if end.exists { end.tap() }
            sleep(1)
        }
        XCTAssertEqual(app.state, .runningForeground, "still running at the end")
    }

    func test5Settings() {
        let app = launch(["-demo", "-route", "more"])
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 15))
        sleep(2)
        snapshot("settings-1-account")
        XCTAssertTrue(app.staticTexts["Sample data, not synced"].exists, "demo says it isn't synced")
        for (id, name) in [("appearance:1", "light"), ("appearance:2", "dark")] {
            let row = app.cells["settings-\(id)"]
            reveal(row, in: app)
            row.tap()
            sleep(1)
            snapshot("settings-2-\(name)")
        }
        let compact = app.switches["toggle-transcript:compact"]
        reveal(compact, in: app)
        if compact.exists {
            compact.tap()
            sleep(1)
            snapshot("settings-3-compact")
            compact.tap()
        }
        let replay = app.cells["settings-replay"]
        reveal(replay, in: app)
        replay.tap()
        sleep(1)
        snapshot("settings-4-replay-toast")
        let archived = app.cells["settings-archived"]
        reveal(archived, in: app)
        archived.tap()
        sleep(2)
        snapshot("settings-5-archived")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        sleep(1)
        let signOut = app.cells["settings-signout"]
        reveal(signOut, in: app)
        signOut.tap()
        sleep(1)
        snapshot("settings-6-sign-out-sheet")
        app.sheets.buttons["Sign Out"].firstMatch.tap()
        XCTAssertTrue(app.buttons["sign-in"].waitForExistence(timeout: 15), "signing out returns to the way in")
        sleep(2)
        snapshot("settings-7-signed-out")
    }
}
