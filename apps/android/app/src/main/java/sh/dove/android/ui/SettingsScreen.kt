package sh.dove.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.ToggleButtonDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import sh.dove.android.core.AppModel
import sh.dove.android.design.ThemeMode
import sh.dove.android.design.DoveIcon
import sh.dove.android.design.DoveIcons
import sh.dove.android.BuildConfig
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsScreen(model: AppModel, onArchived: () -> Unit) {
    val appearance by model.appearance.collectAsState()
    val groupByProject by model.groupByProject.collectAsState()
    var confirmSignOut by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf(false) }
    val workspace by model.workspace.collectAsState()
    val devices = workspace?.devices.orEmpty()
    val list = androidx.compose.foundation.lazy.rememberLazyListState()
    Box(Modifier.fillMaxSize()) {
    LazyColumn(
        Modifier.fillMaxSize(),
        state = list,
        contentPadding = PaddingValues(top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 8.dp, bottom = 140.dp),
    ) {
        item { ScreenHeader("Settings", null) }
        item {
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 12.dp)) {
                Text(model.accountName, style = MaterialTheme.typography.titleMedium)
                Text(model.accountDetail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        section("Appearance")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                Surface(shape = segmentedShapes(0, 2).shape, color = cardColor()) {
                    Column(Modifier.padding(16.dp)) {
                        Text("Theme", style = MaterialTheme.typography.titleMedium)
                        Spacer(Modifier.height(12.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween)) {
                            val modes = listOf(
                                Triple(ThemeMode.System, "System", DoveIcons.Monitor),
                                Triple(ThemeMode.Light, "Light", DoveIcons.Sun),
                                Triple(ThemeMode.Dark, "Dark", DoveIcons.Moon),
                            )
                            modes.forEachIndexed { index, (mode, label, icon) ->
                                ToggleButton(
                                    checked = appearance.mode == mode,
                                    onCheckedChange = { model.setAppearance(appearance.copy(mode = mode)) },
                                    modifier = Modifier.weight(1f).semantics { role = Role.RadioButton },
                                    shapes = when (index) {
                                        0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                                        modes.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                                        else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                                    },
                                ) {
                                    DoveIcon(icon, null, Modifier.size(ToggleButtonDefaults.IconSize))
                                    Spacer(Modifier.size(ToggleButtonDefaults.IconSpacing))
                                    Text(label)
                                }
                            }
                        }
                    }
                }
            }
        }
        section("Sessions")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                SegmentedListItem(
                    onClick = { model.setGroupByProject(!groupByProject) },
                    shapes = segmentedShapes(0, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(DoveIcons.Project) },
                    supportingContent = { Text("Recent sessions grouped by repository") },
                    trailingContent = { Switch(groupByProject, { model.setGroupByProject(it) }) },
                ) { Text("Group by project") }
                SegmentedListItem(
                    onClick = onArchived,
                    shapes = segmentedShapes(1, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(DoveIcons.Archive) },
                    supportingContent = { (workspace?.archived?.size ?: 0).takeIf { it > 0 }?.let { Text("$it sessions") } },
                    trailingContent = { DoveIcon(DoveIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline) },
                ) { Text("Archived sessions") }
            }
        }
        if (devices.isNotEmpty()) {
            section("Devices")
            item {
                Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                    devices.forEachIndexed { i, device ->
                        SegmentedListItem(
                            onClick = {},
                            shapes = segmentedShapes(i, devices.size),
                            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                            leadingContent = {
                                IconTile(
                                    when {
                                        !device.isExecutionHost -> DoveIcons.Phone
                                        device.platform == "linux" -> DoveIcons.Server
                                        else -> DoveIcons.Laptop
                                    },
                                )
                            },
                            supportingContent = {
                                val status = if (device.online) "Online" else "Offline"
                                val self = if (device.isSelf) "This device · " else ""
                                Text("$self${device.platform} · $status")
                            },
                            trailingContent = {
                                if (device.sessionCount > 0u) Text("${device.sessionCount} sessions")
                            },
                        ) { Text(device.name) }
                    }
                }
            }
        }
        section("About")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                SegmentedListItem(
                    onClick = {},
                    shapes = segmentedShapes(0, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(DoveIcons.Info) },
                    supportingContent = { Text("Keplorayx for Android · ${BuildConfig.VERSION_NAME}") },
                ) { Text("Version") }
                SegmentedListItem(
                    onClick = { if (model.isDemo) model.signOut() else confirmSignOut = true },
                    shapes = segmentedShapes(1, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(DoveIcons.Logout, MaterialTheme.colorScheme.errorContainer, MaterialTheme.colorScheme.onErrorContainer) },
                ) { Text(if (model.isDemo) "Leave demo" else "Sign out", color = MaterialTheme.colorScheme.error) }
            }
        }
    }
    StatusBarScrim(scrolled = list.firstVisibleItemIndex > 0 || list.firstVisibleItemScrollOffset > 0)
    }
    if (confirmSignOut) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            icon = { DoveIcon(DoveIcons.Logout, null) },
            title = { Text("Sign out?") },
            text = { Text("Local drafts stay on this device.") },
            confirmButton = {
                androidx.compose.material3.TextButton(onClick = {
                    confirmSignOut = false
                    model.signOut()
                }) { Text("Sign out", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { androidx.compose.material3.TextButton(onClick = { confirmSignOut = false }) { Text("Cancel") } },
        )
    }
}

private fun LazyListScope.section(title: String) {
    item {
        Text(
            title,
            style = MaterialTheme.typography.titleSmallEmphasized,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 28.dp, top = 24.dp, bottom = 8.dp),
        )
    }
}

/** A leading glyph on a tonal rounded tile. */
@Composable
fun IconTile(
    icon: Int,
    container: Color = MaterialTheme.colorScheme.secondaryContainer,
    content: Color = MaterialTheme.colorScheme.onSecondaryContainer,
) {
    Box(Modifier.size(40.dp).clip(RoundedCornerShape(14.dp)).background(container), contentAlignment = Alignment.Center) {
        DoveIcon(icon, null, Modifier.size(22.dp), tint = content)
    }
}
