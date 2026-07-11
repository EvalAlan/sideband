# Keep rules for the Sideband release (R8) build.
#
# Flutter enables R8 for release builds. The QR scanner (mobile_scanner ->
# Google ML Kit barcode) resolves several classes reflectively, so they must
# survive shrinking/obfuscation or the camera fails to start with an obfuscated
# NullPointerException. The `**` (not `*`) is required to cover the
# vision.barcode subpackages.

# ML Kit barcode scanning + its internal barhopper/photos deps.
-keep class com.google.mlkit.** { *; }
-keep class com.google.android.gms.internal.mlkit_vision_barcode.** { *; }
-keep class com.google.android.libraries.barhopper.** { *; }
-keep class com.google.photos.** { *; }

# Optional ML Kit text/barcode modules referenced via reflection; don't warn if
# a given module isn't bundled.
-dontwarn com.google.mlkit.**
-dontwarn com.google.android.gms.internal.mlkit_vision_barcode.**

# mobile_scanner plugin surface.
-keep class dev.steenbakker.mobile_scanner.** { *; }

# Enum valueOf/values are looked up reflectively by ML Kit.
-keepclassmembers class * extends java.lang.Enum {
    <fields>;
    public static **[] values();
    public static ** valueOf(java.lang.String);
}
