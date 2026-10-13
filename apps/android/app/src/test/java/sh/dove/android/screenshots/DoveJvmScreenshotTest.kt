package sh.dove.android.screenshots

import android.content.Intent
import android.os.Looper
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import sh.dove.android.DoveApplication
import sh.dove.android.MainActivity
import sh.dove.android.R
import sh.dove.android.core.LaunchOptions
import sh.dove.android.design.Appearance
import sh.dove.android.design.ThemeMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w411dp-h891dp-xxhdpi")
class DoveJvmScreenshotTest {
    @get:Rule
    val compose = createEmptyComposeRule()

    @Test
    fun signedOutScreen() {
        Screenshots.assumeEnabled()
        val app = ApplicationProvider.getApplicationContext<DoveApplication>()
        val intent = Intent(app, MainActivity::class.java).putExtra("signedout", true)
        val scenario = ActivityScenario.launch<MainActivity>(intent)
        settle()
        compose.onRoot().captureRoboImage(Screenshots.path("01-jvm-sign-in.png"))
        scenario.close()
    }

    @Test
    fun demoListAndTranscript() {
        Screenshots.assumeEnabled()
        Screenshots.assumeHostCore()
        val app = ApplicationProvider.getApplicationContext<DoveApplication>()
        val model = app.model
        val listIntent = Intent(app, MainActivity::class.java)
            .putExtra("demo", true)
            .putExtra("fast", true)
        var scenario = ActivityScenario.launch<MainActivity>(listIntent)
        settleUntil("demo sessions") {
            model.workspace.value?.front?.recent?.isNotEmpty() == true
        }
        assertTrue(model.isDemo)
        compose.onRoot().captureRoboImage(Screenshots.path("02-jvm-demo-list-light.png"))

        model.setAppearance(Appearance(ThemeMode.Dark))
        settle()
        compose.onRoot().captureRoboImage(Screenshots.path("04-jvm-demo-list-dark.png"))

        scenario.close()
        val route = "chat:chat-veil"
        model.boot(LaunchOptions(demo = true, fast = true, route = route))
        val chatIntent = Intent(app, MainActivity::class.java)
            .putExtra("demo", true)
            .putExtra("fast", true)
            .putExtra("route", route)
        scenario = ActivityScenario.launch<MainActivity>(chatIntent)
        settleUntil("demo transcript") { model.row("chat-veil") != null }
        assertNotNull(model.row("chat-veil"))

        model.setAppearance(Appearance(ThemeMode.Light))
        settle()
        compose.onRoot().captureRoboImage(Screenshots.path("03-jvm-demo-transcript-light.png"))
        model.setAppearance(Appearance(ThemeMode.Dark))
        settle()
        compose.onRoot().captureRoboImage(Screenshots.path("05-jvm-demo-transcript-dark.png"))
        scenario.close()
    }

    @Test
    fun launcherIconAsset() {
        Screenshots.assumeEnabled()
        val app = ApplicationProvider.getApplicationContext<DoveApplication>()
        val scenario = ActivityScenario.launch<MainActivity>(Intent(app, MainActivity::class.java))
        scenario.onActivity { activity ->
            activity.setContentView(
                ComposeView(activity).apply {
                    setContent {
                        Box(
                            Modifier.fillMaxSize().background(Color(0xFFF3F3F5)),
                            contentAlignment = Alignment.Center,
                        ) {
                            Box(
                                Modifier.size(180.dp)
                                    .clip(RoundedCornerShape(40.dp))
                                    .background(Color.Black),
                                contentAlignment = Alignment.Center,
                            ) {
                                Image(
                                    painter = painterResource(R.drawable.ic_dove),
                                    contentDescription = "Dove launcher icon",
                                    modifier = Modifier.fillMaxSize(),
                                )
                            }
                        }
                    }
                },
            )
        }
        settle()
        compose.onRoot().captureRoboImage(Screenshots.path("06-jvm-launcher-icon-asset.png"))
        scenario.close()
    }

    private fun settle(ms: Long = 900) {
        val end = System.currentTimeMillis() + ms
        while (System.currentTimeMillis() < end) {
            shadowOf(Looper.getMainLooper()).idleFor(java.time.Duration.ofMillis(50))
            compose.mainClock.advanceTimeBy(50)
            Thread.sleep(10)
        }
        compose.waitForIdle()
    }

    private fun settleUntil(what: String, timeoutMs: Long = 60_000, condition: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (!condition()) {
            check(System.currentTimeMillis() < end) { "Timed out waiting for $what" }
            settle(200)
        }
    }
}
