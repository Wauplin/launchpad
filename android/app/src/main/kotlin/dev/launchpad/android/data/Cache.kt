package dev.launchpad.android.data

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.KSerializer
import java.io.File

/**
 * The cache of lists (and later folio revs and read positions): one JSON file per key in app storage, so a cold
 * start draws at once. Wiped at sign-out and when the device is removed.
 */
class Cache(context: Context) {
    private val dir = File(context.filesDir, "cache-v1").apply { mkdirs() }

    private fun file(key: String) = File(dir, key.replace(Regex("[^A-Za-z0-9._-]"), "_") + ".json")

    fun <T> read(key: String, serializer: KSerializer<T>): T? =
        runCatching { LpJson.decodeFromString(serializer, file(key).readText()) }.getOrNull()

    suspend fun <T> write(key: String, serializer: KSerializer<T>, value: T) = withContext(Dispatchers.IO) {
        runCatching {
            val target = file(key)
            val tmp = File(dir, target.name + ".tmp")
            tmp.writeText(LpJson.encodeToString(serializer, value))
            tmp.renameTo(target)
        }
        Unit
    }

    fun readText(key: String): String? = runCatching { file(key).readText() }.getOrNull()
    suspend fun writeText(key: String, text: String) = withContext(Dispatchers.IO) { runCatching { file(key).writeText(text) }; Unit }

    suspend fun wipe() = withContext(Dispatchers.IO) {
        dir.listFiles()?.forEach { it.delete() }
        Unit
    }
}
