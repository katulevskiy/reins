plugins {
    alias(libs.plugins.android.test)
    alias(libs.plugins.baselineprofile)
}

// Generates the app's baseline profile on a connected device or emulator (see ../README.md); nothing here ships.
android {
    namespace = "dev.reins.android.baselineprofile"
    compileSdk = 37

    defaultConfig {
        minSdk = 31
        targetSdk = 36
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    // The app's distributions: the profile is generated on `full` and merged into src/main for both.
    flavorDimensions += "distribution"
    productFlavors {
        create("full") { dimension = "distribution" }
        create("play") { dimension = "distribution" }
    }

    targetProjectPath = ":app"

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
}

kotlin {
    jvmToolchain(21)
}

baselineProfile {
    useConnectedDevices = true
}

// `play` runs the same code: generating once, on `full`, is enough.
androidComponents {
    beforeVariants(selector().withFlavor("distribution" to "play")) { it.enable = false }
}

dependencies {
    implementation(libs.androidx.test.ext.junit)
    implementation(libs.uiautomator)
    implementation(libs.benchmark.macro.junit4)
}
