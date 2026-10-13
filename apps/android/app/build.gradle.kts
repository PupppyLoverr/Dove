plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

val repoRoot = rootProject.projectDir.resolve("../..").canonicalFile
val meshRoot = repoRoot.resolve("mesh")
val coreOut = meshRoot.resolve("target/android-core")
val generatedIconRoot = layout.buildDirectory.dir("generated/doveIcons").get().asFile
val androidNdkVersion = "27.2.12479018"
val skipCore = providers.gradleProperty("doveSkipCore").isPresent

val buildCore by tasks.registering(Exec::class) {
    description = "Builds Dove's Rust mobile core and generates Kotlin bindings."
    val ndk = System.getenv("ANDROID_NDK_HOME")
        ?: androidComponents.sdkComponents.sdkDirectory.get().dir("ndk/$androidNdkVersion").asFile.path
    val nativeLibraries = coreOut.resolve("jniLibs")
    workingDir = repoRoot
    commandLine("bash", repoRoot.resolve("scripts/android/build-core.sh").path, coreOut.path)
    environment("ANDROID_NDK_HOME", ndk)
    onlyIf {
        !skipCore || nativeLibraries.walkTopDown().any { it.name == "libzeron_mobile.so" } &&
            coreOut.resolve("kotlin").walkTopDown().any { it.extension == "kt" }
    }
}

val generateIcons by tasks.registering(Exec::class) {
    description = "Generates Android vector icons from Dove's shared SVG assets."
    inputs.dir(repoRoot.resolve("apps/dove/assets/icons"))
    inputs.dir(repoRoot.resolve("apps/ios/Zeron/Assets.xcassets/TabIcons"))
    inputs.dir(repoRoot.resolve("apps/ios/Zeron/Assets.xcassets/ToolIcons"))
    inputs.dir(repoRoot.resolve("apps/ios/Zeron/Assets.xcassets/FileIcons"))
    inputs.dir(repoRoot.resolve("apps/android/icons"))
    outputs.dir(generatedIconRoot)
    commandLine(
        "bash",
        repoRoot.resolve("scripts/android/gen-icons.sh").path,
        generatedIconRoot.resolve("assets").path,
        generatedIconRoot.resolve("res").path,
    )
}

android {
    namespace = "sh.dove.android"
    compileSdk = 37
    ndkVersion = androidNdkVersion

    defaultConfig {
        applicationId = "sh.dove.android"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
        unitTests.all { test ->
            test.systemProperty("jna.library.path", meshRoot.resolve("target/mobile").absolutePath)
            test.systemProperty("dove.screenshots", findProperty("doveScreenshots")?.toString() ?: "false")
            test.systemProperty(
                "dove.screenshots.dir",
                findProperty("doveScreenshotsDir")?.toString()
                    ?: layout.buildDirectory.dir("screenshots/jvm").get().asFile.absolutePath,
            )
            test.systemProperty(
                "robolectric.dependency.repo.url",
                "https://maven-central.storage-download.googleapis.com/maven2",
            )
            test.systemProperty("roborazzi.test.record", "true")
            test.maxHeapSize = "3g"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    sourceSets["main"].apply {
        kotlin.directories.add(coreOut.resolve("kotlin").path)
        jniLibs.directories.add(coreOut.resolve("jniLibs").path)
        assets.directories.add(repoRoot.resolve("apps/ios/Zeron/Fonts").path)
        assets.directories.add(generatedIconRoot.resolve("assets").path)
        res.directories.add(generatedIconRoot.resolve("res").path)
    }

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

tasks.named("preBuild") { dependsOn(buildCore, generateIcons) }

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.core.splashscreen)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.browser)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.kotlinx.coroutines.android)
    implementation("${libs.jna.get()}@aar")

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.androidx.test.core)
    testImplementation(libs.androidx.test.ext.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.roborazzi)
    testImplementation(libs.roborazzi.compose)
    testImplementation(libs.roborazzi.junit.rule)
    testImplementation(libs.jna)
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)
}
