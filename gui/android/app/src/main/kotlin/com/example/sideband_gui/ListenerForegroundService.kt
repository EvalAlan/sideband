package com.example.sideband_gui

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Keeps this process foreground-privileged while the embedded Tor client (Rust
 * libsideband.so, driven over dart:ffi from the Flutter/Dart isolate) is expected to keep
 * running in the background.
 *
 * This service intentionally does NOT run the Tor listener itself — that continues to live
 * entirely in the Flutter/Dart process via FFI. Its only job is to hold a foreground service
 * notification so Android doesn't freeze/kill the process while backgrounded.
 *
 * Started/stopped from Dart via MainActivity's "sideband/native" MethodChannel:
 *   - "startForegroundService" -> starts this service (idempotent) and returns null.
 *   - "stopForegroundService"  -> stops this service and returns null.
 */
class ListenerForegroundService : Service() {

    companion object {
        const val CHANNEL_ID = "sideband_listener_channel"
        const val NOTIFICATION_ID = 1001

        const val ACTION_STOP = "com.example.sideband_gui.action.STOP_LISTENER"
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannelIfNeeded()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }

        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        return START_STICKY
    }

    override fun onDestroy() {
        stopForegroundCompat()
        super.onDestroy()
    }

    private fun stopForegroundCompat() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
    }

    private fun createNotificationChannelIfNeeded() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Tor connection",
            NotificationManager.IMPORTANCE_MIN
        ).apply {
            description = "Shows when Sideband is connected to Tor in the background."
            setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(): Notification {
        val contentIntent = packageManager.getLaunchIntentForPackage(packageName)?.let { launchIntent ->
            PendingIntent.getActivity(
                this,
                0,
                launchIntent,
                PendingIntent.FLAG_IMMUTABLE
            )
        }

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }

        builder
            .setContentTitle("Sideband")
            .setContentText("Connected to Tor")
            .setSmallIcon(applicationInfo.icon)
            .setOngoing(true)
            .setContentIntent(contentIntent)
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            // Pre-O: there is no NotificationChannel importance to rely on, so set the
            // legacy priority directly to keep this as unobtrusive as possible.
            @Suppress("DEPRECATION")
            builder.setPriority(Notification.PRIORITY_MIN)
        }
        return builder.build()
    }
}
