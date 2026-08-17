plugins {
    id("com.android.application")
    id("kotlin-android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.example.mesh_ui"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = JavaVersion.VERSION_17.toString()
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.example.mesh_ui"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = 24
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    packaging {
        jniLibs {
            // Keep .so files uncompressed so Android can dlopen() them directly.
            useLegacyPackaging = true
        }
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    // Tell AGP that we also have pre-built JNI libs in src/main/jniLibs.
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}

// ---------------------------------------------------------------------------
// Copy libc++_shared.so from the NDK into the JNI libs directory.
//
// The Rust library is dynamically linked against the NDK's C++ runtime.
// Android does NOT bundle libc++_shared.so automatically, so without this
// step the app crashes at launch with:
//   dlopen failed: library "libc++_shared.so" not found
// ---------------------------------------------------------------------------
tasks.register("copyLibcppShared") {
    doLast {
        val ndkDir = android.ndkDirectory
        // Detect host OS tag (windows-x86_64 locally, linux-x86_64 on CI)
        val prebuiltDir = ndkDir.resolve("toolchains/llvm/prebuilt")
        val hostTag = prebuiltDir.listFiles()?.firstOrNull()?.name
            ?: error("Cannot locate NDK prebuilt directory under $prebuiltDir")

        val abiMap = mapOf(
            "arm64-v8a"   to "aarch64-linux-android",
            "armeabi-v7a" to "arm-linux-androideabi",
            "x86_64"      to "x86_64-linux-android"
        )

        abiMap.forEach { (abi, triple) ->
            val src  = ndkDir.resolve("toolchains/llvm/prebuilt/$hostTag/sysroot/usr/lib/$triple/libc++_shared.so")
            val dest = file("src/main/jniLibs/$abi/libc++_shared.so")
            dest.parentFile.mkdirs()
            if (src.exists()) {
                src.copyTo(dest, overwrite = true)
                println("[copyLibcppShared] Copied libc++_shared.so → $abi")
            } else {
                println("[copyLibcppShared] WARNING: not found at ${src.absolutePath}")
            }
        }
    }
}

// Run before any compilation or resource merging so the .so is present in time.
afterEvaluate {
    tasks.named("preBuild") {
        dependsOn("copyLibcppShared")
    }
}

flutter {
    source = "../.."
}
