# ProGuard rules for WatchCam
-keepclassmembers class * {
    native <methods>;
}

-keep class com.watchcam.android.nativebridge.** { *; }
