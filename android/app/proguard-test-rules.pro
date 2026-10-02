# Instrumented tests against the minified build: keep what the test runner reaches by reflection.
-keep class androidx.tracing.** { *; }
-keep class androidx.test.** { *; }
-dontwarn androidx.test.**
