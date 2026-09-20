import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// The release signing material, when there is any.
//
// The release workflow writes keystore.properties beside settings.gradle from
// repository secrets and deletes it again afterwards, so it exists only for the
// length of that one build; the file and every *.jks and *.keystore are
// gitignored. Keys: storeFile, storePassword, keyAlias, password.
//
// With no file the release build is left unsigned rather than failing, so a
// fork holding no secrets, and anyone building a release locally, still gets an
// APK. An unsigned one cannot install over a signed one, which the workflow
// says out loud when it happens.
val keystoreProperties = Properties().apply {
    val propFile = rootProject.file("keystore.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 36
    namespace = "com.owltransfer.app"
    signingConfigs {
        if (keystoreProperties.containsKey("storeFile")) {
            create("release") {
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                // "password" rather than "keyPassword": it is what the release
                // workflow writes, and the two have to agree.
                keyPassword = keystoreProperties.getProperty("password")
            }
        }
    }
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.owltransfer.app"
        // 30 because the sync folder lives in shared storage and the only
        // permission that reaches it there is MANAGE_EXTERNAL_STORAGE, which
        // arrived in Android 11. Below that the app would have to ask for
        // WRITE_EXTERNAL_STORAGE at runtime and answer a different question
        // about whether storage is reachable, for four Android versions nobody
        // is syncing a folder to in 2026.
        //
        // This is the belt to the braces: tauri.conf.json carries
        // bundle.android.minSdkVersion, which survives a `tauri android init`
        // rewriting this generated file. Keep the two in step.
        minSdk = 30
        // 35 rather than the compileSdk. Android 15 is what the emulator here
        // runs and what the release is tested against; moving the target is a
        // behaviour change, so it moves on purpose rather than with the
        // template.
        targetSdk = 35
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // Null when no keystore.properties was there to read, which leaves
            // the APK unsigned.
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")