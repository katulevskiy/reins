# JNA (UniFFI runtime): accessed reflectively from native code.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-dontwarn java.awt.**
# UniFFI-generated bindings (Structure fields and callbacks are looked up by name).
-keep class dev.reins.core.** { *; }
# ONNX Runtime (Autopilot's model): its native code looks Java classes and fields up by name.
-keep class ai.onnxruntime.** { *; }
# AppFunctions. Library consumer rules already keep @AppFunctionSerializable names,
# AppFunctionSerializableFactory implementors, Schema/Aggregated inventory implementors, and
# AggregatedAppFunctionInvoker implementors. They do not keep this app's service or an inventory
# that implements AppFunctionInventory directly. aapt keeps the service name; this keeps its
# entry points. There is no separate app invoker: the generated service dispatches itself.
-keep class dev.reins.android.platform.assistant.ReinsAppFunctionService { *; }
-keep class * extends androidx.appfunctions.internal.AppFunctionInventory { *; }
-keep class * extends androidx.appfunctions.internal.AppFunctionSerializableFactory { *; }
