package sh.dove.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AuthCallbackTokenTest {
    @Test
    fun prefersTokenAndFallsBackToCoreParsedCode() {
        assertEquals(
            "woo-token",
            authCallbackToken("dove://auth/callback?token=woo-token&code=old&state=nonce", null, null, "nonce"),
        )
        assertEquals(
            "auth-code",
            authCallbackToken("dove://auth/callback?code=auth-code&state=nonce", "auth-code", "nonce", "nonce"),
        )
    }

    @Test
    fun rejectsCallbacksWithoutThePendingState() {
        assertNull(authCallbackToken("dove://auth/callback?token=woo-token", null, null, "nonce"))
        assertNull(authCallbackToken("dove://auth/callback?token=woo-token&state=other", null, null, "nonce"))
        assertNull(authCallbackToken("https://auth/callback?token=woo-token&state=nonce", null, null, "nonce"))
    }
}
