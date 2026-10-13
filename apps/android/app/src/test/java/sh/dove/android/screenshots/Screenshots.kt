package sh.dove.android.screenshots

import org.junit.Assume.assumeTrue
import java.io.File

internal object Screenshots {
    fun assumeEnabled() {
        assumeTrue(
            "JVM screenshots are opt-in; pass -PdoveScreenshots=true",
            System.getProperty("dove.screenshots") == "true",
        )
    }

    fun assumeHostCore() {
        val directory = System.getProperty("jna.library.path")?.let(::File)
        val exists = directory?.listFiles()?.any {
            it.name in setOf("libzeron_mobile.dylib", "libzeron_mobile.so", "zeron_mobile.dll")
        } == true
        assumeTrue("Build the host Rust core before JVM screenshots", exists)
    }

    fun path(name: String): File {
        val directory = File(System.getProperty("dove.screenshots.dir") ?: "build/screenshots/jvm")
        return File(directory, name).apply { parentFile?.mkdirs() }
    }
}
