package sh.dove.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import sh.dove.android.core.LaunchOptions
import sh.dove.android.ui.DoveRoot

/**
 * Launch extras mirror the iOS launch arguments, e.g.
 * `adb shell am start -n sh.dove.android/.MainActivity --ez demo true --es route chat:chat-veil`
 */
class MainActivity : ComponentActivity() {
    private val model get() = (application as DoveApplication).model

    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val extras = intent.extras
        model.boot(
            LaunchOptions(
                demo = extras?.getBoolean("demo") == true,
                fast = extras?.getBoolean("fast") == true,
                longReply = extras?.getBoolean("longreply") == true,
                big = extras?.getBoolean("big") == true,
                huge = extras?.getBoolean("huge") == true,
                noProjects = extras?.getBoolean("noprojects") == true,
                signedOut = extras?.getBoolean("signedout") == true,
                route = extras?.getString("route"),
            ),
        )
        handleCallback(intent)
        setContent { DoveRoot(model) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleCallback(intent)
    }

    private fun handleCallback(intent: Intent?) {
        val data = intent?.data ?: return
        if (data.scheme == "dove") model.handleCallback(data.toString())
    }
}
