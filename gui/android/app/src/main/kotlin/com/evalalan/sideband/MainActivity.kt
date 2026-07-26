package com.evalalan.sideband

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.view.WindowManager
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import java.io.File
import java.io.IOException
import java.net.URLConnection
import java.util.UUID
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {

    companion object {
        // Distinct from mobile_scanner's MobileScannerPermissions.REQUEST_CODE (0x0786 / 1926)
        // so onRequestPermissionsResult dispatch never collides with the camera-permission flow.
        private const val NOTIFICATION_PERMISSION_REQUEST_CODE = 24680
        private const val BLUETOOTH_PERMISSION_REQUEST_CODE = 24681

        const val MESSAGES_CHANNEL_ID = "sideband_messages_channel"
        // Process-owned: the foreground service can keep the Rust listener alive
        // while Flutter recreates its Activity.
        private var bluetoothBridge: BluetoothBridge? = null
    }

    private var pendingNotificationPermissionResult: MethodChannel.Result? = null
    private var pendingBluetoothPermissionResult: MethodChannel.Result? = null

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
                        } catch (e: SecurityException) {
                            result.error("open_file_rejected", e.message, null)
                        } catch (e: Exception) {
                            result.error("open_file_failed", e.message, null)
                        }
                    }

                    "shareFile" -> {
                        val path = call.argument<String>("path") ?: ""
                        try {
                            shareFile(path)
                            result.success(null)
                        } catch (e: SecurityException) {
                            result.error("share_file_rejected", e.message, null)
                        } catch (e: Exception) {
                            result.error("share_file_failed", e.message, null)
                        }
                    }

                    "startForegroundService" -> {
                        try {
                            val intent = Intent(this, ListenerForegroundService::class.java)
                            ContextCompat.startForegroundService(this, intent)
                            result.success(null)
                        } catch (e: Exception) {
                            result.error("start_foreground_service_failed", e.message, null)
                        }
                    }

                    "stopForegroundService" -> {
                        try {
                            val intent = Intent(this, ListenerForegroundService::class.java).apply {
                                action = ListenerForegroundService.ACTION_STOP
                            }
                            startService(intent)
                            result.success(null)
                        } catch (e: Exception) {
                            result.error("stop_foreground_service_failed", e.message, null)
                        }
                    }

                    "requestNotificationPermission" -> {
                        requestNotificationPermission(result)
                    }

                    "requestBluetoothPermissions" -> requestBluetoothPermissions(result)

                    "startBluetoothBridge" -> {
                        val socketPath = call.argument<String>("socketPath")?.trim().orEmpty()
                        val uuidText = call.argument<String>("serviceUuid")?.trim().orEmpty()
                        try {
                            val uuid = UUID.fromString(uuidText)
                            requirePrivateSocketPath(socketPath)
                            if (bluetoothBridge?.matches(socketPath, uuid) == true) {
                                result.success(null)
                                return@setMethodCallHandler
                            }
                            bluetoothBridge?.close()
                            bluetoothBridge = BluetoothBridge(this, socketPath, uuid).also { it.start() }
                            result.success(null)
                        } catch (_: IllegalArgumentException) {
                            result.error("invalid_bluetooth_bridge", "invalid bridge configuration", null)
                        } catch (_: SecurityException) {
                            result.error("bluetooth_permission_denied", "Bluetooth permission denied", null)
                        } catch (_: Exception) {
                            result.error("start_bluetooth_bridge_failed", "Bluetooth bridge could not start", null)
                        }
                    }

                    "stopBluetoothBridge" -> {
                        bluetoothBridge?.close()
                        bluetoothBridge = null
                        result.success(null)
                    }

                    "bluetoothLocalDevice" -> result.success(bluetoothLocalDevice())

                    "showMessageNotification" -> {
                        val title = call.argument<String>("title") ?: "Sideband"
                        val body = call.argument<String>("body") ?: ""
                        val id = call.argument<Int>("id") ?: 0
                        try {
                            showMessageNotification(title, body, id)
                            result.success(null)
                        } catch (e: Exception) {
                            result.error("show_notification_failed", e.message, null)
                        }
                    }

                    "cancelMessageNotifications" -> {
                        try {
                            val manager = getSystemService(NotificationManager::class.java)
                            manager?.cancelAll()
                            result.success(null)
                        } catch (e: Exception) {
                            result.error("cancel_notifications_failed", e.message, null)
                        }
                    }

                    // Block screenshots and screen recording, and hide app
                    // contents in the recent-apps switcher, by toggling
                    // FLAG_SECURE on the window.
                    "setFlagSecure" -> {
                        val enable = call.argument<Boolean>("enable") ?: false
                        try {
                            runOnUiThread {
                                if (enable) {
                                    window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
                                } else {
                                    window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                                }
                            }
                            result.success(null)
                        } catch (e: Exception) {
                            result.error("set_flag_secure_failed", e.message, null)
                        }
                    }

                    else -> result.notImplemented()
                }
            }
    }

    // ---------------------------------------------------------------------
    // File opening, scoped to the app's attachment/download directory only.
    // ---------------------------------------------------------------------

    /**
     * Paths that inbound message text can steer us toward are never trusted as-is:
     * `path` here may originate from attacker-controlled message bodies (see
     * gui/lib/main.dart's parseAttachmentText, which regex-extracts a path out of
     * "[file received: ...]" text). We canonicalize and require the result to live
     * inside the one directory Rust (src/handler.rs) actually writes received files to:
     * "<filesDir>/.sideband/downloads/". Anything else -- including identity.toml,
     * the ratchet/ directory, messages.db, or a path that escapes via ".." -- is rejected.
     */
    private fun openFile(path: String) {
        if (path.isBlank()) {
            throw SecurityException("empty path")
        }

        val requested = File(path).canonicalFile
        val allowedRoot = File(filesDir, ".sideband/downloads").canonicalFile

        if (!isInsideAllowedRoot(requested, allowedRoot)) {
            throw SecurityException("path is outside the allowed attachment directory: $path")
        }
        if (!requested.exists()) {
            throw IllegalArgumentException("file does not exist: $path")
        }

        val uri: Uri = FileProvider.getUriForFile(
            this,
            "${applicationContext.packageName}.fileprovider",
            requested
        )
        val mime = URLConnection.guessContentTypeFromName(requested.name) ?: "application/octet-stream"
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, mime)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        startActivity(Intent.createChooser(intent, requested.name))
    }

    // Share a file (e.g. an exported profile backup) via ACTION_SEND so the user
    // can save it to Drive/Files/etc. Same scoped-path guard as openFile.
    private fun shareFile(path: String) {
        if (path.isBlank()) {
            throw SecurityException("empty path")
        }
        val requested = File(path).canonicalFile
        val allowedRoot = File(filesDir, ".sideband/downloads").canonicalFile
        if (!isInsideAllowedRoot(requested, allowedRoot)) {
            throw SecurityException("path is outside the allowed directory: $path")
        }
        if (!requested.exists()) {
            throw IllegalArgumentException("file does not exist: $path")
        }
        val uri: Uri = FileProvider.getUriForFile(
            this,
            "${applicationContext.packageName}.fileprovider",
            requested
        )
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "application/octet-stream"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        startActivity(Intent.createChooser(intent, "Export Sideband backup"))
    }

    private fun isInsideAllowedRoot(candidate: File, allowedRoot: File): Boolean {
        var dir: File? = candidate
        while (dir != null) {
            if (dir == allowedRoot) return true
            dir = dir.parentFile
        }
        return false
    }

    // ---------------------------------------------------------------------
    // Notifications: "Messages" channel (normal priority) for inbound message
    // notifications while backgrounded. Distinct from the foreground service's
    // own low-priority "Tor connection" channel.
    // ---------------------------------------------------------------------

    private fun ensureMessagesChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            MESSAGES_CHANNEL_ID,
            "Messages",
            NotificationManager.IMPORTANCE_DEFAULT
        ).apply {
            description = "Notifications for new Sideband messages received while backgrounded."
        }
        manager.createNotificationChannel(channel)
    }

    private fun showMessageNotification(title: String, body: String, id: Int) {
        ensureMessagesChannel()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val granted = ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS
            ) == PackageManager.PERMISSION_GRANTED
            if (!granted) {
                // Silently no-op: caller should have requested permission via
                // requestNotificationPermission() first. We don't want to throw here
                // since a missed notification is not fatal.
                return
            }
        }

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, MESSAGES_CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }

        val launchIntent = packageManager.getLaunchIntentForPackage(packageName)
        val contentIntent = launchIntent?.let {
            android.app.PendingIntent.getActivity(
                this,
                id,
                it,
                android.app.PendingIntent.FLAG_IMMUTABLE
            )
        }

        val notification = builder
            .setContentTitle(title)
            .setContentText(body)
            .setSmallIcon(applicationInfo.icon)
            .setAutoCancel(true)
            .apply { if (contentIntent != null) setContentIntent(contentIntent) }
            .build()

        val manager = getSystemService(NotificationManager::class.java)
        manager?.notify(id, notification)
    }

    // ---------------------------------------------------------------------
    // Runtime POST_NOTIFICATIONS permission (API 33+). No-op / auto-granted on
    // older API levels.
    // ---------------------------------------------------------------------

    private fun requestNotificationPermission(result: MethodChannel.Result) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            result.success(true)
            return
        }

        val alreadyGranted = ContextCompat.checkSelfPermission(
            this,
            Manifest.permission.POST_NOTIFICATIONS
        ) == PackageManager.PERMISSION_GRANTED

        if (alreadyGranted) {
            result.success(true)
            return
        }

        if (pendingNotificationPermissionResult != null) {
            result.error("request_in_progress", "a notification permission request is already pending", null)
            return
        }

        pendingNotificationPermissionResult = result
        ActivityCompat.requestPermissions(
            this,
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            NOTIFICATION_PERMISSION_REQUEST_CODE
        )
    }

    private fun requestBluetoothPermissions(result: MethodChannel.Result) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            result.success(true)
            return
        }
        // SCAN as well as CONNECT: with insecure RFCOMM the peer is not bonded,
        // so discovery is how an unpaired contact nearby gets resolved.
        val permissions = arrayOf(
            Manifest.permission.BLUETOOTH_CONNECT,
            Manifest.permission.BLUETOOTH_SCAN,
        )
        if (permissions.all {
                ContextCompat.checkSelfPermission(this, it) == PackageManager.PERMISSION_GRANTED
            }) {
            result.success(true)
            return
        }
        if (pendingBluetoothPermissionResult != null) {
            result.error("request_in_progress", "a Bluetooth permission request is already pending", null)
            return
        }
        pendingBluetoothPermissionResult = result
        ActivityCompat.requestPermissions(this, permissions, BLUETOOTH_PERMISSION_REQUEST_CODE)
    }

    @Suppress("DEPRECATION")
    private fun bluetoothLocalDevice(): String? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.BLUETOOTH_CONNECT) !=
            PackageManager.PERMISSION_GRANTED
        ) return null
        return try {
            val adapter = BluetoothAdapter.getDefaultAdapter() ?: return null
            val address = adapter.address
                ?.takeUnless { it.isBlank() || it == "02:00:00:00:00:00" }
            if (address != null) return address
            adapter.name?.trim()
                ?.takeIf {
                    it.isNotEmpty() && it.length <= 100 && it.all { ch -> !ch.isISOControl() }
                }
                ?.let { "name:$it" }
        } catch (_: SecurityException) {
            null
        }
    }

    private fun requirePrivateSocketPath(path: String) {
        if (path.isBlank()) throw IllegalArgumentException("empty socket path")
        val socket = File(path).canonicalFile
        val privateRoot = filesDir.canonicalFile
        if (!isInsideAllowedRoot(socket, privateRoot)) {
            throw SecurityException("bridge socket must be in private app storage")
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray
    ) {
        // IMPORTANT: always call super so Flutter plugins (e.g. mobile_scanner's camera
        // permission flow, registered via the ActivityPluginBinding's own listener) keep
        // receiving their callbacks. Our own request code (24680) is chosen well clear of
        // mobile_scanner's MobileScannerPermissions.REQUEST_CODE (0x0786 / 1926) so the two
        // never collide.
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)

        if (requestCode == NOTIFICATION_PERMISSION_REQUEST_CODE) {
            val granted = grantResults.isNotEmpty() &&
                grantResults[0] == PackageManager.PERMISSION_GRANTED
            pendingNotificationPermissionResult?.success(granted)
            pendingNotificationPermissionResult = null
        } else if (requestCode == BLUETOOTH_PERMISSION_REQUEST_CODE) {
            val granted = grantResults.isNotEmpty() &&
                grantResults.all { it == PackageManager.PERMISSION_GRANTED }
            pendingBluetoothPermissionResult?.success(granted)
            pendingBluetoothPermissionResult = null
        }
    }

    override fun onDestroy() {
        pendingBluetoothPermissionResult?.error("activity_destroyed", "activity destroyed", null)
        pendingBluetoothPermissionResult = null
        pendingNotificationPermissionResult?.error("activity_destroyed", "activity destroyed", null)
        pendingNotificationPermissionResult = null
        super.onDestroy()
    }
}
