package sh.dove.android.net

import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.QueueEditAction
import uniffi.zeron_core.QueueEditLease
import uniffi.zeron_core.SendRequest
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.TranscriptView
import uniffi.zeron_core.UserInputAnswer

class ChatBridge internal constructor(
    private val client: CoreClient,
    private val session: SessionHandle,
    private val chatId: String,
) {
    fun attachTranscript(view: TranscriptView) = view.attach(client, chatId)
    fun composer() = session.composer()
    fun setViewAttached(attached: Boolean) = session.setViewAttached(attached)
    fun retryDelivery() = session.retryDelivery()
    fun respondInput(requestId: String, answers: List<UserInputAnswer>) = session.respondInput(requestId, answers)
    suspend fun beginQueuedEdit(id: String, owner: String) = session.beginQueuedEdit(id, owner)
    suspend fun renewQueuedEdit(lease: QueueEditLease) = session.renewQueuedEdit(lease)
    suspend fun finishQueuedEdit(lease: QueueEditLease, action: QueueEditAction, body: String?) =
        session.finishQueuedEdit(lease, action, body)
    fun interrupt() = session.interrupt()
    fun send(request: SendRequest) = session.send(request)
    suspend fun deliverQueuedNow(id: String) = session.deliverQueuedNow(id)
    fun moveQueuedBy(id: String, offset: Int) = session.moveQueuedBy(id, offset)
    suspend fun removeQueued(id: String) = session.removeQueued(id)
}
