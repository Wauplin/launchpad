# kotlinx.serialization ships its own consumer rules; the generated serializers of the app's types are kept by them.
-dontwarn org.commonmark.ext.gfm.strikethrough.Strikethrough
-keepattributes *Annotation*, InnerClasses
# The folio reader's bridge: methods called from JavaScript.
-keepclassmembers class * { @android.webkit.JavascriptInterface <methods>; }
