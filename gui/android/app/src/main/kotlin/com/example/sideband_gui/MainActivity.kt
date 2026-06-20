package com.example.sideband_gui

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "sideband/native")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "profilePath" -> result.success("${filesDir.absolutePath}/.sideband")
                    else -> result.notImplemented()
                }
            }
    }
}
