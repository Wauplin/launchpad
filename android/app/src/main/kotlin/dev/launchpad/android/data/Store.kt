package dev.launchpad.android.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.Serializable
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Everything that gives access: encrypted with a Keystore key before it reaches DataStore, never backed up. */
@Serializable
data class Secrets(
    /** The device secret (`X-Launchpad-Device`); empty until Home created the device. */
    val device: String = "",
    val hfAccess: String = "",
    /** Unix ms. */
    val hfExpiresAt: Long = 0,
    val hfRefresh: String = "",
)

/** The linked Home (one per install). Not secret. */
@Serializable
data class HomeLink(
    /** `https://<sub>.hf.space`. */
    val origin: String = "",
    /** `owner/name` of the Space. */
    val space: String = "",
    val deviceId: String = "",
    val deviceName: String = "",
    val account: String = "",
)

@Serializable
data class Settings(
    /** system, light or dark. */
    val theme: String = "system",
    val workspace: String = "",
    /** The Ask box's last target: `bot:<id>` or `agent`. */
    val askTarget: String = "",
)

private val Context.dataStore by preferencesDataStore(name = "launchpad")

/**
 * The app's small durable state: secrets (Keystore AES-GCM), the Home link and this phone's settings. Read once at
 * start (a few hundred bytes), then served from memory; every change is written through.
 */
class Store(private val context: Context) {
    private val kSecrets = stringPreferencesKey("secrets")
    private val kLink = stringPreferencesKey("link")
    private val kSettings = stringPreferencesKey("settings")

    private val _secrets = MutableStateFlow(Secrets())
    private val _link = MutableStateFlow(HomeLink())
    private val _settings = MutableStateFlow(Settings())
    val secrets: StateFlow<Secrets> = _secrets
    val link: StateFlow<HomeLink> = _link
    val settings: StateFlow<Settings> = _settings

    init {
        runBlocking {
            val prefs = runCatching { context.dataStore.data.first() }.getOrNull()
            prefs?.get(kSecrets)?.let { text -> runCatching { LpJson.decodeFromString(Secrets.serializer(), decrypt(text)) }.getOrNull() }?.let { _secrets.value = it }
            prefs?.get(kLink)?.let { text -> runCatching { LpJson.decodeFromString(HomeLink.serializer(), text) }.getOrNull() }?.let { _link.value = it }
            prefs?.get(kSettings)?.let { text -> runCatching { LpJson.decodeFromString(Settings.serializer(), text) }.getOrNull() }?.let { _settings.value = it }
        }
    }

    /** Linked: Home created this phone's device and the app holds its secret. */
    val linked: Boolean get() = _link.value.origin.isNotEmpty() && _secrets.value.device.isNotEmpty()

    suspend fun setSecrets(value: Secrets) {
        _secrets.value = value
        val text = encrypt(LpJson.encodeToString(Secrets.serializer(), value))
        context.dataStore.edit { it[kSecrets] = text }
    }

    suspend fun setLink(value: HomeLink) {
        _link.value = value
        context.dataStore.edit { it[kLink] = LpJson.encodeToString(HomeLink.serializer(), value) }
    }

    suspend fun setSettings(value: Settings) {
        _settings.value = value
        context.dataStore.edit { it[kSettings] = LpJson.encodeToString(Settings.serializer(), value) }
    }

    /** Sign out, or the device was removed: secrets and the link go; the theme stays. */
    suspend fun wipe() {
        _secrets.value = Secrets()
        _link.value = HomeLink()
        _settings.value = _settings.value.copy(workspace = "", askTarget = "")
        context.dataStore.edit {
            it.remove(kSecrets)
            it.remove(kLink)
            it[kSettings] = LpJson.encodeToString(Settings.serializer(), _settings.value)
        }
        runCatching { KeyStore.getInstance(KEYSTORE).apply { load(null) }.deleteEntry(KEY_ALIAS) }
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private fun encrypt(plain: String): String {
        val cipher = Cipher.getInstance(AES_GCM).apply { init(Cipher.ENCRYPT_MODE, key()) }
        val sealed = cipher.doFinal(plain.toByteArray())
        return Base64.encodeToString(cipher.iv + sealed, Base64.NO_WRAP)
    }

    private fun decrypt(text: String): String {
        val bytes = Base64.decode(text, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(AES_GCM).apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12)) }
        return String(cipher.doFinal(bytes, 12, bytes.size - 12))
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val KEY_ALIAS = "launchpad-secrets"
        const val AES_GCM = "AES/GCM/NoPadding"
    }
}
