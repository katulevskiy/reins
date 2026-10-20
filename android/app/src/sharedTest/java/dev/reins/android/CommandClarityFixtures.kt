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

    // Exact old-core fallback for this command's unsupported redirects, retained for before/after evidence.
    fun presentation() = CommandPresentation(
        "Review a shell command", "", emptyList(),
        listOf("Reins cannot safely shorten this invocation. Inspect the full invocation, including its setup and shell syntax."),
        raw,
    )

    fun label() = TestData.label("terminal", "run", "Run", raw, code = true).copy(command = presentation())
    fun view() = TestData.askView(
        detail = "Command:\n$raw\n\nIn: $cwd",
        rule = AskRuleView("Keys and certificates", "*.keystore", keystore, "key-files", false, false),
    ).let { it.copy(waitUntil = null, noStanding = true, ask = it.ask!!.copy(uncertain = true)) }

    fun entry() = TestData.entry(7, "write", "released", service = "desktop", op = "ask", label = label(),
        decidedBy = "bypass", request = TestData.asked(raw, "Build and test release APKs", "Codex", cwd,
            "Reading a secret file", 1_700_000_000)).copy(connectionLabel = "honeypot", account = null)
}
