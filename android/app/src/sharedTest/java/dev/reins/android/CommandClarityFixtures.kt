package dev.reins.android

import dev.reins.core.AskRuleView
import dev.reins.core.CommandPresentation

/** The reported release build, with synthetic credential sentinels. No command is executed by these UI fixtures. */
object CommandClarityFixtures {
    const val cwd = "/home/user/Documents/dev/reins"
    const val keystore = "/home/user/.cache/reins-permission-controls/local-test.keystore"
    const val log = "/tmp/reins-controls-bounded-smoke-compile.log"
    const val raw = "env JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_HOME=/home/user/Android/Sdk " +
        "ANDROID_SDK_ROOT=/home/user/Android/Sdk ./gradlew :app:assembleFullRelease :app:assembleFullReleaseAndroidTest " +
        "-Preins.testBuildType=release -Preins.testRunner=dev.reins.android.TrustTestRunner " +
        "-Preins.prebuiltNativeDir=/home/user/.cache/reins-permission-controls/native-complete " +
        "-Preins.releaseKeystore=$keystore -Preins.releaseKeystorePassword=fixture-secret-sentinel " +
        "-Preins.releaseKeyAlias=androiddebugkey -Preins.releaseKeyPassword=fixture-secret-sentinel " +
        "-Pkotlin.incremental=false --no-configuration-cache > $log 2>&1"

    // Same raw input as the baseline; this is display metadata, never authority or an executed command.
    fun presentation() = CommandPresentation(
        "Build Android release and test APKs",
        "./gradlew :app:assembleFullRelease :app:assembleFullReleaseAndroidTest > $log 2>&1",
        listOf("Environment wrapper: env", "JAVA_HOME=•••• (value hidden)", "ANDROID_HOME=•••• (value hidden)", "ANDROID_SDK_ROOT=•••• (value hidden)",
            "gradlew options/configuration (9): -Preins.testBuildType=•••• -Preins.testRunner=•••• -Preins.prebuiltNativeDir=•••• -Preins.releaseKeystore=•••• -Preins.releaseKeystorePassword=•••• -Preins.releaseKeyAlias=•••• -Preins.releaseKeyPassword=•••• -Pkotlin.incremental=•••• --no-configuration-cache"),
        listOf("Redirected output replaces $log (>).", "Standard error joins standard output (2>&1).",
            "References signing configuration or a signing key; no read or signature is proven.",
            "Gradle runs project build scripts/plugins. Options and properties can change behavior and use this process's permissions and credentials."),
        raw,
    )

    fun label() = TestData.label("terminal", "run", "Run", raw, code = true).copy(command = presentation())
    fun view() = TestData.askView(
        detail = "Command:\n$raw\n\nIn: $cwd",
        rule = AskRuleView("Keys and certificates", "*.keystore", keystore, "key-files", false, false),
    ).let { it.copy(waitUntil = null, noStanding = true, ask = it.ask!!.copy(uncertain = true)) }

    fun entry() = TestData.entry(7, "write", "released", service = "desktop", op = "ask", label = label(),
        decidedBy = "bypass", request = TestData.asked(raw, "Build and test release APKs", "Codex", cwd,
            "References signing configuration or a signing key", 1_700_000_000)).copy(connectionLabel = "honeypot", account = null)
}
