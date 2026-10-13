package sh.dove.android.core

import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets

internal fun authCallbackToken(
    url: String,
    coreCode: String?,
    coreState: String?,
    expectedState: String?,
): String? {
    if (expectedState.isNullOrBlank()) return null
    val uri = runCatching { URI(url) }.getOrNull() ?: return null
    if (!uri.scheme.equals("dove", ignoreCase = true) ||
        !uri.host.equals("auth", ignoreCase = true) ||
        uri.path != "/callback"
    ) {
        return null
    }

    val query = uri.rawQuery.orEmpty().split('&').mapNotNull { pair ->
        val (key, value) = pair.split('=', limit = 2).let { it[0] to it.getOrElse(1) { "" } }
        runCatching {
            URLDecoder.decode(key, StandardCharsets.UTF_8.name()) to URLDecoder.decode(value, StandardCharsets.UTF_8.name())
        }.getOrNull()
    }.toMap()
    val returnedState = coreState ?: query["state"]
    if (returnedState != expectedState) return null
    return query["token"]?.takeIf(String::isNotBlank)
        ?: coreCode?.takeIf(String::isNotBlank)
        ?: query["code"]?.takeIf(String::isNotBlank)
}
