package sh.dove.android.ui

import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import sh.dove.android.R
import sh.dove.android.core.AppModel

@Composable
fun SignInScreen(model: AppModel) {
    val context = LocalContext.current
    val error by model.signInError.collectAsState()
    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Image(
            painterResource(R.drawable.ic_dove),
            null,
            Modifier.size(72.dp),
            colorFilter = ColorFilter.tint(MaterialTheme.colorScheme.onSurface),
        )
        Spacer(Modifier.height(24.dp))
        Text("Keplorayx", style = MaterialTheme.typography.headlineLarge)
        Spacer(Modifier.height(8.dp))
        Text(
            "Follow and steer your coding agents from anywhere.",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(40.dp))
        Button(
            onClick = {
                val url = model.beginSignIn()
                CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, Uri.parse(url))
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Sign in with Woozlit")
        }
        Spacer(Modifier.height(12.dp))
        OutlinedButton(onClick = model::startDemo, modifier = Modifier.fillMaxWidth()) {
            Text("Explore the demo")
        }
        error?.let {
            Spacer(Modifier.height(20.dp))
            Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.Center)
        }
    }
}
