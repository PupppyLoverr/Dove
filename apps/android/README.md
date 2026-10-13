# Dove for Android

Dove for Android lets you follow and steer Dove sessions from an Android device. Sign in with Woozlit or open the built-in demo.

## Build and run

Install the Android SDK, NDK `27.2.12479018`, Rust targets `aarch64-linux-android` and `x86_64-linux-android`, and `cargo-ndk`. From this directory, build, test, and lint with:

```sh
./gradlew assembleDebug
./gradlew testDebugUnitTest
./gradlew lintDebug
```

The first build compiles Dove's Rust mobile core, generates UniFFI bindings and Android icons, and packages the `arm64-v8a` and `x86_64` libraries. Generated files stay under `mesh/target/android-core`.

Install and launch the demo:

```sh
adb install app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n sh.dove.android/.MainActivity --ez demo true
```

To open the sign-in screen or a demo transcript:

```sh
adb shell am start -n sh.dove.android/.MainActivity --ez signedout true
adb shell am start -n sh.dove.android/.MainActivity --ez demo true --es route chat:chat-veil
```

Launch extras also accept `fast`, `longreply`, `big`, `huge`, and `noprojects` as booleans. `route` accepts a session route such as `chat:chat-veil`.

## Sign-in and demo

Sign-in uses Woozlit only. The app checks the OAuth state in the `dove://auth/callback` link and stores credentials with Android Keystore encryption. Demo mode loads the Rust core's existing demo fixtures and doesn't create a test account.

## App architecture

Rust decides what to paint; Kotlin paints. Dove's mesh core owns session behavior, authentication, and the view data sent to Jetpack Compose. `net/DoveClient.kt` and `net/ChatBridge.kt` are the Kotlin boundary to that core. UniFFI generates the Kotlin bindings and native libraries during the Android build.

The Kotlin registry replica would be a second source of truth. Cosmos's `RegistryBridge` only nudged the host and could not deliver messages durably, so sends and interrupts go through the core's command ledger.

## Pairing

Dove's mobile core exposes no pairing API. Offer, list, and revoke are host IPC in `mesh/crates/rpc/src/lib.rs`; the app signs in with Woozlit only and has no code or QR entry.

## Screenshot tests

Screenshot tests are opt-in; from `apps/android`, run `./gradlew --console=plain :app:testDebugUnitTest -PdoveScreenshots=true -PdoveScreenshotsDir=/Users/devin/work/shots --tests 'sh.dove.android.screenshots.DoveJvmScreenshotTest'`.
