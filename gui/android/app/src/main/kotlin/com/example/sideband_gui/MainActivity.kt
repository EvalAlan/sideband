package com.example.sideband_gui

import android.content.Intent
import android.net.Uri
import androidx.core.content.FileProvider
import java.io.File
import java.net.URLConnection
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
                    "openFile" -> {
                        val path = call.argument<String>("path") ?: ""
                        try {
                            openFile(path)
                            result.success(null)
                        } catch (e: Exception) {
                            result.error("open_file_failed", e.message, null)
                        }
                    }
                    else -> result.notImplemented()
                }
            }
    }

    private fun openFile(path: String) {
        val file = File(path)
        require(file.exists()) { "file does not exist: $path" }
        val uri: Uri = FileProvider.getUriForFile(
            this,
            "${applicationContext.packageName}.fileprovider",
            file
        )
        val mime = URLConnection.guessContentTypeFromName(file.name) ?: "application/octet-stream"
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, mime)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        startActivity(Intent.createChooser(intent, file.name))
    }
}
