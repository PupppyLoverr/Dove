package sh.dove.android.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject
import uniffi.zeron_core.AuthTokens
import uniffi.zeron_core.Credentials

/**
 * Credentials and the signed-in account's display identity, in app-private
 * storage (the client itself only knows ids). Tokens are sealed with an
 * Android Keystore key (the iOS Keychain's counterpart): the key never
 * leaves the keystore, so a copied preferences file can't be replayed.
 */
class CredentialStore(context: Context) {
    private val prefs = context.getSharedPreferences("credentials", Context.MODE_PRIVATE)

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return generator.generateKey()
    }

    private fun seal(plain: String): String {
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, key()) }
        val sealed = cipher.iv + cipher.doFinal(plain.toByteArray())
        return Base64.encodeToString(sealed, Base64.NO_WRAP)
    }

    private fun open(sealed: String): String? = runCatching {
        val bytes = Base64.decode(sealed, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12))
        String(cipher.doFinal(bytes, 12, bytes.size - 12))
    }.getOrNull()

    private fun storedJson(): String? {
        return prefs.getString("sealed", null)?.let(::open)
    }

    fun stored(): Credentials? {
        val json = storedJson()?.let { runCatching { JSONObject(it) }.getOrNull() } ?: return null
        return when (json.optString("kind")) {
            "woozlit" -> Credentials.Woozlit(
                json.getString("userId"),
                AuthTokens(json.getString("access"), json.getString("refresh")),
            )
            else -> null
        }
    }

    fun store(credentials: Credentials) {
        val json = when (credentials) {
            is Credentials.Woozlit -> JSONObject()
                .put("kind", "woozlit")
                .put("userId", credentials.userId)
                .put("access", credentials.tokens.accessToken)
                .put("refresh", credentials.tokens.refreshToken)
            is Credentials.WorkOs -> return
            is Credentials.Dev -> return
            is Credentials.Demo -> return
        }
        prefs.edit().putString("sealed", seal(json.toString())).apply()
    }

    fun updateTokens(tokens: AuthTokens) {
        val current = stored() as? Credentials.Woozlit ?: return
        store(Credentials.Woozlit(current.userId, tokens))
    }

    fun clear() {
        prefs.edit().clear().apply()
    }

    data class Profile(val name: String? = null, val email: String? = null)

    var profile: Profile
        get() = Profile(prefs.getString("name", null), prefs.getString("email", null))
        set(value) {
            prefs.edit().putString("name", value.name).putString("email", value.email).remove("orgName").apply()
        }

    private companion object {
        const val KEY_ALIAS = "dove-credentials"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
    }
}
