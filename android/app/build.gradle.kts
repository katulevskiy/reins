import javax.inject.Inject

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// google-services.json is optional (plan Decision 18). Copy it in from outside the repo when
// available; apply the plugin only when the file is present so the app builds without Firebase.
val googleServicesSource = file(
    providers.gradleProperty("reins.googleServicesJson")
        .getOrElse("${System.getProperty("user.home")}/.config/reins/google-services.json"),
)
val googleServicesTarget = file("google-services.json")
if (!googleServicesTarget.exists() && googleServicesSource.isFile) {
    googleServicesSource.copyTo(googleServicesTarget)
}
val reinsNdkVersion = "27.2.12479018"
val hasFirebase = googleServicesTarget.isFile
if (hasFirebase) {
    apply(plugin = "com.google.gms.google-services")
}

android {
    namespace = "dev.reins.android"
    compileSdk = 37
    ndkVersion = reinsNdkVersion

    defaultConfig {
        applicationId = "com.reins2fa.app"
        minSdk = 31
        targetSdk = 36
        // scripts/release-android.sh sets these (versionCode = release time in minutes, so every release is newer) and
        // reads the default versionName from the line below; keep its shape.
        versionCode = providers.gradleProperty("reins.versionCode").map(String::toInt).getOrElse(1)
        versionName = providers.gradleProperty("reins.versionName").getOrElse("0.1.0")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Shown next to the version in Settings ("0.1.0-202610010115-a9643bcb" for releases, "dev" for local builds).
        val buildId = providers.gradleProperty("reins.build").getOrElse("dev")
        buildConfigField("String", "BUILD_ID", "\"$buildId\"")
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
        buildConfigField("boolean", "HAS_FIREBASE", hasFirebase.toString())
        // The server new accounts and sign-ins use unless the user picks another: the hosted server
        // (gradle.properties) unless -Preins.defaultServer=https://your.server says otherwise.
        val defaultServer = providers.gradleProperty("reins.defaultServer").get().trimEnd('/')
        buildConfigField("String", "DEFAULT_SERVER", "\"$defaultServer\"")
        // Telegram's application credentials (my.telegram.org). They identify this app to Telegram, not the user, and
        // live in ~/.gradle/gradle.properties, never in the repository. Without them Telegram shows "needs setup".
        val telegramId = providers.gradleProperty("reins.telegramApiId").getOrElse("0")
        val telegramHash = providers.gradleProperty("reins.telegramApiHash").getOrElse("")
        buildConfigField("int", "TELEGRAM_API_ID", telegramId)
        buildConfigField("String", "TELEGRAM_API_HASH", "\"$telegramHash\"")
    }

    // Where the app comes from. Both have the same application id: one is installed at a time, and as Google Play
    // signs `play` with its own key, switching between them means uninstalling (see PLAY_STORE.md).
    flavorDimensions += "distribution"
    productFlavors {
        // The APK on reins2fa.com (scripts/release-android.sh): text messages, and it updates itself.
        create("full") {
            dimension = "distribution"
            isDefault = true
            buildConfigField("boolean", "HAS_SMS", "true")
            buildConfigField("boolean", "SELF_UPDATE", "true")
            // Where the in-app updater looks for new releases (written by scripts/release-android.sh).
            val site = providers.gradleProperty("reins.site").get().trimEnd('/')
            val updateUrl = providers.gradleProperty("reins.updateUrl").getOrElse("$site/releases/android/latest.json")
            buildConfigField("String", "UPDATE_URL", "\"$updateUrl\"")
        }
        // Google Play: no SMS permissions (only default SMS apps may have them) and no updater or
        // REQUEST_INSTALL_PACKAGES (Play installs the updates). What only `full` declares is in src/full.
        create("play") {
            dimension = "distribution"
            buildConfigField("boolean", "HAS_SMS", "false")
            buildConfigField("boolean", "SELF_UPDATE", "false")
            buildConfigField("String", "UPDATE_URL", "\"\"")
        }
    }

    // Native libraries (the core, ONNX Runtime) are compressed in the APK: a much smaller download, unpacked once at
    // install.
    packaging { jniLibs { useLegacyPackaging = true } }

    // -Preins.testBuildType=release runs the instrumented tests against the minified (R8) build.
    providers.gradleProperty("reins.testBuildType").orNull?.let { testBuildType = it }

    // The SHA-1 registered with Google/Firebase is that of ~/.android/debug.keystore. AGP may pick another
    // debug keystore (it follows ANDROID_USER_HOME), so pin it when it exists.
    signingConfigs.getByName("debug") {
        val registered = file("${System.getProperty("user.home")}/.android/debug.keystore")
        if (registered.isFile) storeFile = registered
    }
    // The Google Play upload key, when ~/.gradle/gradle.properties names it (reins.uploadKeystore,
    // reins.uploadKeystorePassword, reins.uploadKeyAlias, reins.uploadKeyPassword): it signs `playRelease`
    // (see below). Without it that bundle has the debug key, which is fine for checking it locally; Play refuses it.
    providers.gradleProperty("reins.uploadKeystore").orNull?.let { keystore ->
        signingConfigs.create("upload") {
            storeFile = file(keystore)
            storePassword = providers.gradleProperty("reins.uploadKeystorePassword").get()
            keyAlias = providers.gradleProperty("reins.uploadKeyAlias").get()
            keyPassword = providers.gradleProperty("reins.uploadKeyPassword").get()
        }
    }
    // The key of the published `full` APK, when a Gradle property or the environment names it (the GitHub release
    // workflow uses the environment): reins.releaseKeystore / REINS_RELEASE_KEYSTORE,
    // reins.releaseKeystorePassword / REINS_RELEASE_KEYSTORE_PASSWORD, reins.releaseKeyAlias /
    // REINS_RELEASE_KEY_ALIAS, reins.releaseKeyPassword / REINS_RELEASE_KEY_PASSWORD (default: the keystore
    // password). It signs `fullRelease` (see below). Without it `fullRelease` has the debug key, and
    // scripts/release-android.sh re-signs it with apksigner.
    fun releaseSetting(property: String, env: String) =
        providers.gradleProperty(property).orElse(providers.environmentVariable(env)).orNull?.takeIf { it.isNotEmpty() }
    releaseSetting("reins.releaseKeystore", "REINS_RELEASE_KEYSTORE")?.let { keystore ->
        val storePass = releaseSetting("reins.releaseKeystorePassword", "REINS_RELEASE_KEYSTORE_PASSWORD")
            ?: error("reins.releaseKeystore is set, but not its password (REINS_RELEASE_KEYSTORE_PASSWORD)")
        signingConfigs.create("release") {
            storeFile = file(keystore)
            storePassword = storePass
            keyAlias = releaseSetting("reins.releaseKeyAlias", "REINS_RELEASE_KEY_ALIAS")
                ?: error("reins.releaseKeystore is set, but not the key alias (REINS_RELEASE_KEY_ALIAS)")
            keyPassword = releaseSetting("reins.releaseKeyPassword", "REINS_RELEASE_KEY_PASSWORD") ?: storePass
        }
    }

    // FLAG_SECURE (no screenshots, blank in recents) on the screens that show secrets: on in release builds, off in
    // debug builds so the Robolectric flow and screenshot tests can capture them. -Preins.secureScreens=true|false
    // overrides both.
    val secureScreensOverride = providers.gradleProperty("reins.secureScreens").orNull?.toBooleanStrict()
    fun secureScreens(default: Boolean) = (secureScreensOverride ?: default).toString()

    buildTypes {
        debug {
            buildConfigField("boolean", "SECURE_SCREENS", secureScreens(default = false))
        }
        release {
            buildConfigField("boolean", "SECURE_SCREENS", secureScreens(default = true))
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // MVP: only the debug keystore's SHA-1 is registered with Firebase / Google Cloud. scripts/release-android.sh
            // re-signs `full` with the app's key; a `play` bundle is signed with the upload key (PLAY_STORE.md).
            signingConfig = signingConfigs.getByName("debug")
            testProguardFiles("proguard-test-rules.pro")
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }


    testOptions {
        unitTests.isReturnDefaultValues = true
        unitTests.isIncludeAndroidResources = true
        // -Dreins.screenshots=/dir renders the design-review screenshots (see ScreenshotsTest).
        unitTests.all { test -> System.getProperty("reins.screenshots")?.let { test.systemProperty("reins.screenshots", it) } }
    }
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        freeCompilerArgs.addAll(
            "-opt-in=androidx.compose.material3.ExperimentalMaterial3Api",
            "-opt-in=androidx.compose.material3.ExperimentalMaterial3ExpressiveApi",
        )
    }
}

// ---- Rust core: cargo-ndk builds the shared libraries, uniffi-bindgen generates the Kotlin bindings. ----

val repoRoot: File = rootProject.projectDir.parentFile
val rustProfile = providers.gradleProperty("reins.rustProfile").getOrElse("release")
val cargoBin = "${System.getProperty("user.home")}/.cargo/bin"

/** Builds `reins-core` for the shipped ABIs with 16 KB page alignment. cargo is incremental, so this always runs. */
abstract class CargoNdkTask : DefaultTask() {
    @get:Internal abstract val workspaceRoot: DirectoryProperty
    @get:Input abstract val profile: Property<String>
    @get:Input abstract val ndkDir: Property<String>
    @get:Input abstract val cargoBinDir: Property<String>
    @get:OutputDirectory abstract val outputDir: DirectoryProperty
    @get:Inject abstract val exec: ExecOperations

    init {
        outputs.upToDateWhen { false }
    }

    @TaskAction
    fun build() {
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        val profileName = profile.get()
        // cargo's `dev` profile lives in `debug/`; every other profile has its own directory name.
        val cargoProfile = if (profileName == "debug") "dev" else profileName
        exec.exec {
            workingDir = workspaceRoot.get().asFile
            environment("PATH", "${cargoBinDir.get()}:${System.getenv("PATH")}")
            environment("ANDROID_NDK_HOME", ndkDir.get())
            environment("CARGO_ENCODED_RUSTFLAGS", "-Clink-arg=-Wl,-z,max-page-size=16384")
            commandLine(
                "${cargoBinDir.get()}/cargo", "ndk", "-t", "arm64-v8a", "-t", "x86_64", "--platform", "31", "-o", out.absolutePath,
                "build", "--profile", cargoProfile, "-p", "reins-core", "--lib", "--locked",
            )
        }
    }
}

/**
 * Generates the Kotlin bindings from a host build of the same crate (the UniFFI metadata does not depend on
 * the target, and host builds keep their symbols whatever profile the Android libraries use).
 */
abstract class UniffiBindgenTask : DefaultTask() {
    @get:Internal abstract val workspaceRoot: DirectoryProperty
    @get:Input abstract val cargoBinDir: Property<String>
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val rustSources: ConfigurableFileCollection
    @get:OutputDirectory abstract val outputDir: DirectoryProperty
    @get:Inject abstract val exec: ExecOperations

    @TaskAction
    fun generate() {
        val root = workspaceRoot.get().asFile
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        val path = "${cargoBinDir.get()}:${System.getenv("PATH")}"
        exec.exec {
            workingDir = root
            environment("PATH", path)
            commandLine("${cargoBinDir.get()}/cargo", "build", "-p", "reins-core", "--lib", "--features", "bindgen", "--locked")
        }
        // The host's own build of the core (bindgen reads its metadata): .dylib on macOS, .so elsewhere, in
        // CARGO_TARGET_DIR when that is set.
        val hostLibrary = if (System.getProperty("os.name").startsWith("Mac")) "libreins_core.dylib" else "libreins_core.so"
        val targetDir = System.getenv("CARGO_TARGET_DIR")?.takeIf { it.isNotEmpty() }?.let { root.resolve(it) } ?: root.resolve("target")
        exec.exec {
            workingDir = root
            environment("PATH", path)
            commandLine(
                "${cargoBinDir.get()}/cargo", "run", "-q", "-p", "reins-core", "--features", "bindgen", "--bin", "uniffi-bindgen",
                "--locked", "--", "generate", "--library", targetDir.resolve("debug/$hostLibrary").absolutePath,
                "--language", "kotlin", "--no-format", "--out-dir", out.absolutePath,
            )
        }
    }
}

// Both distributions use the same native core and bindings. Generate them once per Gradle invocation;
// separate per-variant tasks repeated the host build, bindgen and two ABI builds during unit tests.
val sharedCargo = tasks.register<CargoNdkTask>("cargoNdk") {
    workspaceRoot.set(repoRoot)
    profile.set(rustProfile)
    ndkDir.set(androidComponents.sdkComponents.sdkDirectory.map { it.asFile.resolve("ndk/$reinsNdkVersion").absolutePath })
    cargoBinDir.set(cargoBin)
    outputDir.set(layout.buildDirectory.dir("rustJniLibs/shared"))
}

val sharedBindgen = tasks.register<UniffiBindgenTask>("uniffiBindgen") {
    workspaceRoot.set(repoRoot)
    cargoBinDir.set(cargoBin)
    rustSources.from(
        fileTree(repoRoot) {
            include("crates/reins-core/src/**", "crates/reins-core/Cargo.toml", "crates/reins-core/uniffi.toml")
            include("crates/reins-proto/src/**", "crates/reins-proto/Cargo.toml")
            include("crates/reins-policy/src/**", "crates/reins-policy/Cargo.toml", "Cargo.toml", "Cargo.lock")
        },
    )
    outputDir.set(layout.buildDirectory.dir("generated/uniffi/shared/kotlin"))
}

androidComponents {
    // The release build type's debug key would otherwise win over a flavor's signing config.
    onVariants(selector().withBuildType("release").withFlavor("distribution" to "play")) { variant ->
        android.signingConfigs.findByName("upload")?.let { variant.signingConfig.setConfig(it) }
    }
    onVariants(selector().withBuildType("release").withFlavor("distribution" to "full")) { variant ->
        android.signingConfigs.findByName("release")?.let { variant.signingConfig.setConfig(it) }
    }
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(sharedCargo, CargoNdkTask::outputDir)
        variant.sources.kotlin?.addGeneratedSourceDirectory(sharedBindgen, UniffiBindgenTask::outputDir)
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.biometric)
    implementation(libs.work.runtime.ktx)
    implementation(libs.firebase.messaging)
    implementation(libs.play.services.auth)
    // Google's QR code scanner (its own camera screen in Play services; no camera permission) for pairing codes.
    implementation(libs.play.services.code.scanner)
    implementation(libs.coroutines.android)
    implementation(libs.androidsvg)
    // Custom Tabs for MCP servers' sign-in pages.
    implementation(libs.androidx.browser)
    implementation(libs.jna) { artifact { type = "aar" } }
    // Autopilot's model runs on the phone (the core does everything else; see autopilot/OnnxModelRuntime).
    implementation(libs.onnxruntime.android)

    testImplementation(libs.junit)
    testImplementation(libs.coroutines.test)
    testImplementation(libs.work.testing)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.core)
    testImplementation(libs.androidx.test.ext.junit)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.espresso.core)
    androidTestImplementation(libs.coroutines.test)
    debugImplementation(libs.compose.ui.test.manifest)
}
