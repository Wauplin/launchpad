package dev.launchpad.android.update

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.app.NotificationCompat
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import dev.launchpad.android.BuildConfig
import dev.launchpad.android.R
import dev.launchpad.android.data.LpJson
import dev.launchpad.android.push.Push
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import okhttp3.OkHttpClient
import okhttp3.Request
import java.util.concurrent.TimeUnit

/** A newer APK on GitHub: its name and download address. */
data class Update(val name: String, val url: String, val page: String)

/**
 * The app updates from GitHub releases (build.yml's `android` job): a release build reads the latest release, a main
 * build the `main-build` prerelease. Every 6 hours, and from This phone → Check for updates. The newest APK whose
 * name is not this build's is offered; Android installs it only when the same key signed it.
 */
object UpdateCheck {
    @Serializable private data class Asset(val name: String = "", val browser_download_url: String = "", val created_at: String = "")
    @Serializable private data class Release(val html_url: String = "", val assets: List<Asset> = emptyList())

    private val http by lazy { OkHttpClient.Builder().callTimeout(30, TimeUnit.SECONDS).build() }

    /** dev builds (local Gradle) never look for updates. */
    val enabled: Boolean get() = BuildConfig.CHANNEL == "release" || BuildConfig.CHANNEL == "main"

    fun schedule(context: Context) {
        if (!enabled) return
        val work = PeriodicWorkRequestBuilder<Worker>(6, TimeUnit.HOURS).build()
        WorkManager.getInstance(context).enqueueUniquePeriodicWork("update-check", ExistingPeriodicWorkPolicy.KEEP, work)
    }

    /** The newer APK, or null when this build is the newest. Throws when GitHub does not answer. */
    suspend fun check(): Update? = withContext(Dispatchers.IO) {
        val path = if (BuildConfig.CHANNEL == "main") "releases/tags/main-build" else "releases/latest"
        val request = Request.Builder().url("https://api.github.com/repos/${BuildConfig.GITHUB_REPO}/$path")
            .header("Accept", "application/vnd.github+json").header("User-Agent", "launchpad-android").build()
        val text = http.newCall(request).execute().use { r ->
            if (!r.isSuccessful) throw java.io.IOException("GitHub answered ${r.code}")
            r.body.string()
        }
        val release = LpJson.decodeFromString(Release.serializer(), text)
        val newest = release.assets.filter { it.name.startsWith("launchpad-android-") && it.name.endsWith(".apk") }.maxByOrNull { it.created_at } ?: return@withContext null
        if (newest.name == "launchpad-android-${BuildConfig.VERSION_NAME}.apk") null else Update(newest.name, newest.browser_download_url, release.html_url)
    }

    fun notify(context: Context, update: Update) {
        val open = PendingIntent.getActivity(context, 7, Intent(Intent.ACTION_VIEW, Uri.parse(update.url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK), PendingIntent.FLAG_IMMUTABLE)
        val n = NotificationCompat.Builder(context, Push.CH_UPDATES)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle("A newer Launchpad app is available")
            .setContentText(update.name.removePrefix("launchpad-android-").removeSuffix(".apk") + " · tap to download")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        runCatching { context.getSystemService(NotificationManager::class.java).notify("update", 0, n) }
    }

    class Worker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
        override suspend fun doWork(): Result {
            val update = runCatching { check() }.getOrElse { return Result.retry() } ?: return Result.success()
            val prefs = applicationContext.getSharedPreferences("update", Context.MODE_PRIVATE)
            // Offered once per build.
            if (prefs.getString("offered", null) != update.name) {
                notify(applicationContext, update)
                prefs.edit().putString("offered", update.name).apply()
            }
            return Result.success()
        }
    }
}
