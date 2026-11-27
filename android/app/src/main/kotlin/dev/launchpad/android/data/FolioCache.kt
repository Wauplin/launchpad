package dev.launchpad.android.data

import dev.launchpad.android.Graph
import kotlinx.serialization.Serializable
import java.io.File
import java.security.MessageDigest

/** A saved answer of Home: what the folio reader's page needs to draw again without a connection. */
class CachedResponse(val contentType: String?, val bytes: ByteArray)

/** Where the owner stopped reading a folio: the page and the heading at the top of the screen. */
@Serializable
data class FolioPosition(val page: String? = null, val anchor: String? = null)

/**
 * The folio reader's offline copy: the successful GET answers of the embed page (entry and assets, shared by every
 * folio) and of each folio's read routes, one folder per folio. Network first; the copy is read only when Home
 * does not answer, so it always holds the last rev that was opened. Bounded: the [MAX_FOLIOS] folios opened last
 * are kept, and page assets not used for [ASSET_DAYS] days go (a Home update renames them). Wiped at sign-out and
 * when the device is removed, with the rest of the cache.
 */
object FolioCache {
    private const val MAX_FOLIOS = 8
    private const val MAX_ENTRY = 8 * 1024 * 1024
    private const val ASSET_DAYS = 14L
    private const val SHARED = "_page"

    private val root: File get() = File(Graph.context.filesDir, "folio-cache-v1").apply { mkdirs() }

    private fun safe(s: String) = s.replace(Regex("[^A-Za-z0-9._-]"), "_")
    private fun hash(s: String) = MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }.take(40)

    /**
     * The folder and name of a request, or null when it is not kept. The page's own files are keyed by path (its
     * address carries the reading position); a folio's routes by path and query (the rev).
     */
    private fun slot(ws: String, slug: String, pathAndQuery: String): File? {
        val path = pathAndQuery.substringBefore('?')
        val folio = "/api/folios/$ws/$slug"
        return when {
            path.startsWith("/folio-embed/") -> File(File(root, SHARED), hash(path))
            path == "/api/workspaces" || path == "/api/projects" || path == "/api/bots" -> File(File(root, SHARED), hash(pathAndQuery))
            path == folio || path.startsWith("$folio/") -> File(File(root, safe("$ws.$slug")), hash(pathAndQuery))
            else -> null
        }
    }

    fun put(ws: String, slug: String, pathAndQuery: String, contentType: String?, bytes: ByteArray) {
        if (bytes.size > MAX_ENTRY) return
        val file = slot(ws, slug, pathAndQuery) ?: return
        runCatching {
            file.parentFile?.mkdirs()
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.writeBytes(bytes)
            tmp.renameTo(file)
            File(file.parentFile, file.name + ".type").writeText(contentType ?: "")
        }
    }

    fun get(ws: String, slug: String, pathAndQuery: String): CachedResponse? {
        val file = slot(ws, slug, pathAndQuery) ?: return null
        return runCatching {
            if (!file.isFile) return null
            file.setLastModified(System.currentTimeMillis())
            CachedResponse(File(file.parentFile, file.name + ".type").takeIf { it.isFile }?.readText()?.ifEmpty { null }, file.readBytes())
        }.getOrNull()
    }

    /** A folio was opened: it is the newest, and what is over the bounds goes. */
    fun opened(ws: String, slug: String) {
        runCatching {
            val mine = File(root, safe("$ws.$slug")).apply { mkdirs() }
            mine.setLastModified(System.currentTimeMillis())
            root.listFiles { f -> f.isDirectory && f.name != SHARED }?.sortedByDescending { it.lastModified() }?.drop(MAX_FOLIOS)?.forEach { it.deleteRecursively() }
            val old = System.currentTimeMillis() - ASSET_DAYS * 86_400_000
            File(root, SHARED).listFiles { f -> !f.name.endsWith(".type") && f.lastModified() < old }?.forEach {
                File(it.parentFile, it.name + ".type").delete()
                it.delete()
            }
        }
    }

    fun wipe() {
        runCatching { root.deleteRecursively() }
    }

    // ------------------------------------------------------------ reading positions (in the lists' cache, wiped with it)

    private fun key(ws: String, slug: String) = "folio-position-$ws-$slug"

    fun position(ws: String, slug: String): FolioPosition =
        Graph.cache.read(key(ws, slug), FolioPosition.serializer()) ?: FolioPosition()

    suspend fun setPosition(ws: String, slug: String, position: FolioPosition) =
        Graph.cache.write(key(ws, slug), FolioPosition.serializer(), position)
}
