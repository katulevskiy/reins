# Instrumented tests against the minified build: keep what the test runner reaches by reflection.
-keep class androidx.tracing.** { *; }
-keep class androidx.test.** { *; }
-dontwarn androidx.test.**
-keep class dev.reins.android.core.CoreProvider { *; }
-keep interface dev.reins.android.core.CoreFactory { *; }
-keep class dev.reins.android.platform.ScreenLock { *; }
# The separately compiled test APK calls library APIs that R8 could otherwise inline out of the target APK.
# Keep those API signatures while still optimizing their implementations and all production screen code.
-keep,allowoptimization class kotlin.** { public protected *; }
-keep,allowoptimization class kotlinx.coroutines.** { public protected *; }
-keep,allowoptimization class dev.reins.android.design.Timers { public *; }
-keep,allowoptimization class dev.reins.android.design.Providers { public *; }
-keep,allowoptimization class dev.reins.android.design.Provider { public *; }
-keep,allowoptimization class dev.reins.android.ui.activity.ActivityLogicKt { public *; }
-keep,allowoptimization class dev.reins.android.ui.common.AskSentenceKt { public *; }
-keep,allowoptimization class dev.reins.android.ui.common.UntrustedKt { public *; }
