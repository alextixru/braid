plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "org.braid.vpn"
    // receive_sharing_intent требует compileSdk 37 и выше.
    compileSdk = maxOf(flutter.compileSdkVersion, 37)
    ndkVersion = flutter.ndkVersion

    compileOptions {
        // flutter_local_notifications требует core library desugaring.
        isCoreLibraryDesugaringEnabled = true
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "org.braid.vpn"
        // Android 10+ (ист. 3): Builder.setMetered, isAlwaysOn, NetworkCallback без устаревших API.
        minSdk = 29
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        // Нативные библиотеки — braid_ffi (cargokit) и libawg.so (engines/awg-go) — есть
        // только для этих ABI; остальные в APK не кладутся.
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    sourceSets {
        getByName("main") {
            // libawg.so (cgo c-shared, `make libawg-android` с ANDROID_NDK_HOME):
            // engines/awg-go/dist/android/<abi>/libawg.so. libbraid_ffi.so линкуется с ней.
            jniLibs.srcDirs(rootProject.file("../../engines/awg-go/dist/android"))
        }
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
            // Имена методов сервиса и native-методов — контракт с Rust (см. proguard-rules.pro).
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.5")
    // Unit-тесты описания интерфейса, домена из share-меню и идентификатора сети
    // (`./gradlew :app:testDebugUnitTest`, CI — тикет 12).
    testImplementation("junit:junit:4.13.2")
    // Настоящий org.json для JVM-тестов: в android.jar он — заглушка.
    testImplementation("org.json:json:20250107")
}

// cargokit берёт список платформ у Flutter-плагина (`-Ptarget-platform`; без флага —
// ещё и armeabi-v7a, в debug — x86), а libawg.so и цели Rust есть только для
// arm64-v8a и x86_64. Ограничиваем сборку braid_ffi этими двумя.
gradle.projectsEvaluated {
    rootProject.allprojects.forEach { subproject ->
        subproject.tasks.matching { it.name.startsWith("cargokitCargoBuild") }.configureEach {
            setProperty("targetPlatforms", listOf("android-arm64", "android-x64"))
        }
    }
}
