package sh.dove.android.net

import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import uniffi.zeron_core.ClientEvent
import uniffi.zeron_core.ClientListener
import uniffi.zeron_core.Connectivity
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreConfig
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.Credentials
import uniffi.zeron_core.NewSession
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.WorkspaceSnapshot
import uniffi.zeron_core.authWoozlitExchange as exchangeWoozlitToken
import uniffi.zeron_core.woozlitSignInUrl as coreWoozlitSignInUrl

class DoveClient(
    config: CoreConfig,
    credentials: Credentials,
    private val onClientEvent: (ClientEvent) -> Unit,
) {
    private val _workspace = MutableStateFlow<WorkspaceSnapshot?>(null)
    val workspace: StateFlow<WorkspaceSnapshot?> = _workspace.asStateFlow()

    private val _connectivity = MutableStateFlow<Connectivity?>(null)
    val connectivity: StateFlow<Connectivity?> = _connectivity.asStateFlow()

    private val _sessionEvents = MutableSharedFlow<String>(extraBufferCapacity = 256)
    val sessionEvents: SharedFlow<String> = _sessionEvents.asSharedFlow()

    private val listener = object : ClientListener {
        override fun onEvent(event: ClientEvent) {
            when (event) {
                is ClientEvent.WorkspaceChanged -> refreshWorkspace()
                is ClientEvent.SessionChanged -> _sessionEvents.tryEmit(event.chatId)
                is ClientEvent.ComposerChanged -> _sessionEvents.tryEmit(event.chatId)
                is ClientEvent.ConnectivityChanged -> _connectivity.value = event.connectivity
                else -> Unit
            }
            onClientEvent(event)
        }
    }

    private val client = CoreClient(config, credentials, listener)

    init {
        refreshWorkspace()
    }

    fun isDemo() = client.isDemo()
    fun refreshWorkspace() {
        _workspace.value = client.workspace()
    }

    fun preloadSessions() = client.preloadSessions()
    fun onForeground() = client.onForeground()
    fun onBackground() = client.onBackground()
    fun setNetworkOnline(online: Boolean) = client.setNetworkOnline(online)
    fun shutdown() = client.shutdown()

    fun archiveSession(id: String) = client.archiveSession(id)
    fun unarchiveSession(id: String) = client.unarchiveSession(id)
    fun renameSession(id: String, title: String) = client.renameSession(id, title)
    fun markSeen(id: String) = client.markSeen(id)
    fun pinSession(id: String) = client.pinSession(id)
    fun unpinSession(id: String) = client.unpinSession(id)
    fun setSectionCollapsed(id: String, collapsed: Boolean) = client.setSectionCollapsed(id, collapsed)
    fun assignSection(id: String, sectionId: String?) = client.assignSection(id, sectionId)
    fun movePin(id: String, after: String?, before: String?) = client.movePin(id, after, before)
    fun createSection(name: String) = client.createSection(name)
    fun renameSection(id: String, name: String) = client.renameSection(id, name)
    fun deleteSection(id: String) = client.deleteSection(id)
    fun sessionRow(id: String): SessionRow? = client.sessionRow(id)
    fun createSession(session: NewSession) = client.createSession(session)
    fun openChat(chatId: String) = ChatBridge(client, client.openSession(chatId), chatId)

    suspend fun listFolders(deviceId: String, path: String?) = client.listFolders(deviceId, path)
    suspend fun createProject(deviceId: String, path: String, gitDetected: Boolean) =
        client.createProject(deviceId, path, gitDetected)
    suspend fun listHarnesses(deviceId: String) = client.listHarnesses(deviceId)
    suspend fun listModels(deviceId: String, harness: String) = client.listModels(deviceId, harness)
    suspend fun listRefs(deviceId: String, repoPath: String) = client.listRefs(deviceId, repoPath)
    fun sessionConfig(chatId: String) = client.sessionConfig(chatId)
    fun setSessionConfig(chatId: String, config: ChatConfig) = client.setSessionConfig(chatId, config)
    fun search(query: String, limit: UInt) = client.search(query, limit)
    suspend fun searchFiles(deviceId: String, chatId: String?, spaceId: String?, query: String) =
        client.searchFiles(deviceId, chatId, spaceId, query)
    suspend fun readAttachment(deviceId: String, path: String) = client.readAttachment(deviceId, path)

    companion object {
        fun signInUrl(state: String) = coreWoozlitSignInUrl(state)
        suspend fun exchangeWoozlit(token: String) = exchangeWoozlitToken(token)
    }
}
