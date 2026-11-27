plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// Everything the build bakes in comes from the environment (GitHub Actions: build.yml), with defaults that build anywhere.
fun env(name: String, default: String = ""): String = System.getenv(name)?.takeIf { it.isNotBlank() } ?: default
fun quoted(value: String) = "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

// The release key (GitHub secrets, decoded to a file by the workflow). Without it, release builds are signed with the
// debug key, so a build without secrets still installs (but never updates an app signed with the release key).
val keystoreFile = env("ANDROID_KEYSTORE_FILE")
val releaseKey = keystoreFile.isNotEmpty() && file(keystoreFile).exists()

android {
    namespace = "dev.launchpad.android"
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }

    defaultConfig {
        applicationId = "dev.launchpad.android"
        minSdk = 29
        targetSdk = 37
        versionCode = env("LAUNCHPAD_ANDROID_VERSION_CODE", "1").toInt()
        versionName = env("LAUNCHPAD_ANDROID_VERSION", "0.0.0-dev")
        // The shared public Hugging Face OAuth app (PKCE, no secret): the same as the Chrome extension and the desktop companion.
        buildConfigField("String", "HF_CLIENT_ID", quoted(env("LAUNCHPAD_HF_CLIENT_ID")))
        // "release" or "main": which GitHub release the update check reads.
        buildConfigField("String", "CHANNEL", quoted(env("LAUNCHPAD_ANDROID_CHANNEL", "dev")))
        buildConfigField("String", "COMMIT", quoted(env("LAUNCHPAD_ANDROID_COMMIT")))
        buildConfigField("String", "GITHUB_REPO", quoted(env("LAUNCHPAD_GITHUB_REPO", "Wauplin/launchpad")))
        // Firebase Cloud Messaging (the owner's project). All empty: the app never starts Firebase and gets no push.
        buildConfigField("String", "FIREBASE_PROJECT_ID", quoted(env("LAUNCHPAD_FIREBASE_PROJECT_ID")))
        buildConfigField("String", "FIREBASE_APP_ID", quoted(env("LAUNCHPAD_FIREBASE_APP_ID")))
        buildConfigField("String", "FIREBASE_API_KEY", quoted(env("LAUNCHPAD_FIREBASE_API_KEY")))
        buildConfigField("String", "FIREBASE_SENDER_ID", quoted(env("LAUNCHPAD_FIREBASE_SENDER_ID")))
    }

    signingConfigs {
        if (releaseKey) {
            create("release") {
                storeFile = file(keystoreFile)
                storePassword = env("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = env("ANDROID_KEY_ALIAS")
                keyPassword = env("ANDROID_KEY_PASSWORD", env("ANDROID_KEYSTORE_PASSWORD"))
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName(if (releaseKey) "release" else "debug")
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources.excludes += setOf("META-INF/{AL2.0,LGPL2.1}", "META-INF/LICENSE*", "META-INF/NOTICE*")
    }
}

kotlin {
    jvmToolchain {
        // Any installed JDK 17 or newer builds it; bytecode targets 17.
    }
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.process)
    implementation(libs.browser)
    implementation(libs.datastore.preferences)
    implementation(libs.work.runtime)
    implementation(libs.webkit)
    implementation(libs.core.ktx)
    implementation(libs.fragment)
    implementation(libs.firebase.messaging)
    implementation(libs.okhttp)
    implementation(libs.serialization.json)
    implementation(libs.coroutines.android)
    implementation(libs.commonmark)
    implementation(libs.commonmark.gfm.tables)
    implementation(libs.commonmark.gfm.strikethrough)
    implementation(libs.commonmark.autolink)
}
