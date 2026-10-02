# JNA (UniFFI runtime): accessed reflectively from native code.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-dontwarn java.awt.**
# UniFFI-generated bindings (Structure fields and callbacks are looked up by name).
-keep class dev.rewarden.core.** { *; }
# ONNX Runtime (Autopilot's model): its native code looks Java classes and fields up by name.
-keep class ai.onnxruntime.** { *; }
