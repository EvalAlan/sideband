package com.evalalan.sideband

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.content.pm.PackageManager
import android.net.LocalSocket
import android.net.LocalSocketAddress
import android.os.Build
import android.util.Base64
import androidx.core.content.ContextCompat
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.EOFException
import java.io.IOException
import java.io.InputStream
import java.nio.charset.StandardCharsets
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Pure RFCOMM framing helpers, kept independent of Android Bluetooth APIs for JVM tests. */
internal object RfcommFrameCodec {
    // BTP permits 4 MiB; leave a small allowance for its envelope while still enforcing
    // a hard allocation bound at this carrier boundary.
    const val MAX_FRAME_BYTES: Int = 4 * 1024 * 1024 + 64 * 1024

    fun read(input: InputStream): ByteArray {
        val data = DataInputStream(input)
        val length = try {
            data.readInt()
        } catch (e: EOFException) {
            throw e
        }
        if (length < 0 || length > MAX_FRAME_BYTES) {
            throw IOException("invalid RFCOMM frame length")
        }
        return ByteArray(length).also(data::readFully)
    }

    fun write(output: java.io.OutputStream, payload: ByteArray) {
        if (payload.size > MAX_FRAME_BYTES) throw IOException("RFCOMM frame too large")
        DataOutputStream(output).apply {
            writeInt(payload.size)
            write(payload)
            flush()
        }
    }
}

/**
 * Android-only RFCOMM carrier adapter. Rust remains the protocol owner: this class only
 * moves opaque BTP/BSP wire bytes between a private filesystem Unix socket and RFCOMM.
 */
internal class BluetoothBridge(
    context: Context,
    private val socketPath: String,
    private val serviceUuid: UUID,
) : Closeable {
    companion object {
        private const val SERVICE_NAME = "Sideband"
        private const val MAX_COMMAND_BYTES = 6 * 1024 * 1024
        private const val RECONNECT_DELAY_MS = 500L
        private const val MAX_SESSIONS = 8
    }

    private val appContext = context.applicationContext
    private val running = AtomicBoolean(false)
    private val localExecutor = Executors.newSingleThreadExecutor()
    private val acceptExecutor = Executors.newSingleThreadExecutor()
    private val dialExecutor = Executors.newSingleThreadExecutor()
    private val ioExecutor = Executors.newFixedThreadPool(MAX_SESSIONS)
    private val writeExecutor = Executors.newSingleThreadExecutor()
    private val localWriteLock = Any()
    private val sessionLock = Any()
    private val sessions = mutableMapOf<String, BluetoothSocket>()
    private val inFlightDials = mutableMapOf<Long, BluetoothSocket>()
    private val nextInboundSession = AtomicLong(1)

    @Volatile private var localSocket: LocalSocket? = null
    @Volatile private var serverSocket: BluetoothServerSocket? = null

    fun start() {
        if (!running.compareAndSet(false, true)) return
        requireBluetoothPermission()
        val adapter = BluetoothAdapter.getDefaultAdapter()
            ?: throw IllegalStateException("Bluetooth is unavailable")
        if (!adapter.isEnabled) throw IllegalStateException("Bluetooth is disabled")

        try {
            serverSocket = adapter.listenUsingRfcommWithServiceRecord(SERVICE_NAME, serviceUuid)
        } catch (e: SecurityException) {
            running.set(false)
            throw SecurityException("Bluetooth permission denied", e)
        } catch (e: IOException) {
            running.set(false)
            throw IOException("could not open Bluetooth listener", e)
        }
        acceptExecutor.execute(::acceptLoop)
        localExecutor.execute(::localSocketLoop)
    }

    fun matches(path: String, uuid: UUID): Boolean =
        running.get() && socketPath == path && serviceUuid == uuid

    private fun localSocketLoop() {
        while (running.get()) {
            val socket = LocalSocket()
            try {
                socket.connect(LocalSocketAddress(socketPath, LocalSocketAddress.Namespace.FILESYSTEM))
                localSocket = socket
                readCommands(socket.inputStream)
            } catch (_: IOException) {
                // Rust may still be starting, or may recreate its private socket. Retry without
                // exposing filesystem paths or peer identifiers in UI-facing errors/logcat.
            } finally {
                if (localSocket === socket) localSocket = null
                socket.closeQuietly()
            }
            if (running.get()) {
                try {
                    Thread.sleep(RECONNECT_DELAY_MS)
                } catch (_: InterruptedException) {
                    Thread.currentThread().interrupt()
                    return
                }
            }
        }
    }

    private fun readCommands(input: InputStream) {
        while (running.get()) {
            val line = readBoundedLine(input, MAX_COMMAND_BYTES) ?: return
            if (line.isEmpty()) continue
            val command = try {
                JSONObject(line)
            } catch (_: Exception) {
                continue
            }
            when (command.optString("type")) {
                "dial" -> handleDial(command)
                "write" -> handleWrite(command)
                "ack" -> handleAck(command)
                "cancel" -> handleCancel(command)
                "close" -> closeSession(command.optString("session_id"))
            }
        }
    }

    private fun handleDial(command: JSONObject) {
        val id = command.optLong("id", -1)
        val sessionId = command.optString("session_id")
        if (id < 0 || sessionId.isEmpty()) return
        val address = command.optString("device")
        val uuidText = command.optString("uuid")
        if (!BluetoothAdapter.checkBluetoothAddress(address) && !address.startsWith("name:")) {
            sendResult(id, false, "invalid Bluetooth device")
            return
        }
        val uuid = try {
            UUID.fromString(uuidText)
        } catch (_: IllegalArgumentException) {
            sendResult(id, false, "invalid Bluetooth service UUID")
            return
        }
        dialExecutor.execute {
            var socket: BluetoothSocket? = null
            try {
                requireBluetoothPermission()
                val adapter = BluetoothAdapter.getDefaultAdapter()
                    ?: throw IOException("Bluetooth unavailable")
                val device: BluetoothDevice = resolveDevice(adapter, address)
                socket = device.createRfcommSocketToServiceRecord(uuid)
                synchronized(sessionLock) { inFlightDials[id] = socket }
                socket.connect()
                synchronized(sessionLock) { inFlightDials.remove(id) }
                if (!running.get() || !installRfcommSocket(sessionId, socket)) {
                    socket.closeQuietly()
                    sendResult(id, false, "Bluetooth bridge stopped")
                } else {
                    socket = null
                    sendResult(id, true, null)
                }
            } catch (_: SecurityException) {
                sendResult(id, false, "Bluetooth permission denied")
            } catch (_: Exception) {
                // Never reflect an exception containing a remote MAC back to Rust/UI.
                sendResult(id, false, "Bluetooth connection failed")
            } finally {
                synchronized(sessionLock) { inFlightDials.remove(id) }
                socket?.closeQuietly()
            }
        }
    }

    private fun resolveDevice(adapter: BluetoothAdapter, hint: String): BluetoothDevice {
        if (BluetoothAdapter.checkBluetoothAddress(hint)) return adapter.getRemoteDevice(hint)
        val name = hint.removePrefix("name:")
        if (name.isEmpty()) throw IOException("invalid Bluetooth device")
        val matches = adapter.bondedDevices.filter { it.name == name }
        if (matches.size != 1) throw IOException("Bluetooth device is not uniquely paired")
        return matches.single()
    }

    private fun handleWrite(command: JSONObject) {
        val id = command.opt("id") ?: return
        val sessionId = command.optString("session_id")
        val encoded = command.optString("wire_b64")
        val wire = try {
            Base64.decode(encoded, Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            sendResult(id, false, "invalid wire payload")
            return
        }
        if (wire.size > RfcommFrameCodec.MAX_FRAME_BYTES) {
            sendResult(id, false, "wire payload too large")
            return
        }
        writeExecutor.execute {
            val socket = synchronized(sessionLock) { sessions[sessionId] }
            if (socket == null || !socket.isConnected) {
                sendResult(id, false, "no Bluetooth connection")
                return@execute
            }
            try {
                synchronized(socket) {
                    RfcommFrameCodec.write(socket.outputStream, wire)
                }
                sendResult(id, true, null)
            } catch (_: IOException) {
                closeSession(sessionId)
                sendResult(id, false, "Bluetooth write failed")
            }
        }
    }

    private fun handleAck(command: JSONObject) {
        val sessionId = command.optString("session_id")
        val wire = try {
            Base64.decode(command.optString("wire_b64"), Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            return
        }
        if (wire.size != 16) return
        writeExecutor.execute {
            val socket = synchronized(sessionLock) { sessions[sessionId] } ?: return@execute
            try {
                synchronized(socket) {
                    RfcommFrameCodec.write(socket.outputStream, wire)
                }
            } catch (_: IOException) {
                closeSession(sessionId)
            }
        }
    }

    private fun handleCancel(command: JSONObject) {
        val id = command.optLong("id", -1)
        val sessionId = command.optString("session_id")
        if (id >= 0) synchronized(sessionLock) { inFlightDials[id] }?.closeQuietly()
        closeSession(sessionId)
    }

    private fun acceptLoop() {
        while (running.get()) {
            val accepted = try {
                serverSocket?.accept() ?: return
            } catch (_: IOException) {
                if (running.get()) close()
                return
            } catch (_: SecurityException) {
                if (running.get()) close()
                return
            }
            val sessionId = "in-${nextInboundSession.getAndIncrement()}"
            if (!installRfcommSocket(sessionId, accepted)) accepted.closeQuietly()
        }
    }

    private fun installRfcommSocket(sessionId: String, socket: BluetoothSocket): Boolean {
        synchronized(sessionLock) {
            if (!running.get() || sessions.size >= MAX_SESSIONS || sessions.containsKey(sessionId)) {
                return false
            }
            sessions[sessionId] = socket
        }
        ioExecutor.execute { readRfcommFrames(sessionId, socket) }
        return true
    }

    private fun readRfcommFrames(sessionId: String, socket: BluetoothSocket) {
        try {
            while (running.get() && synchronized(sessionLock) { sessions[sessionId] === socket }) {
                val wire = RfcommFrameCodec.read(socket.inputStream)
                sendLocal(
                    JSONObject()
                        .put("type", "inbound")
                        .put("session_id", sessionId)
                        .put("wire_b64", Base64.encodeToString(wire, Base64.NO_WRAP))
                )
            }
        } catch (_: IOException) {
            // Disconnects, malformed lengths, and truncation all close this carrier stream.
        } finally {
            socket.closeQuietly()
            synchronized(sessionLock) {
                if (sessions[sessionId] === socket) sessions.remove(sessionId)
            }
        }
    }

    private fun closeSession(sessionId: String) {
        val socket = synchronized(sessionLock) { sessions.remove(sessionId) }
        socket?.closeQuietly()
    }

    private fun sendResult(id: Any, ok: Boolean, error: String?) {
        val message = JSONObject().put("type", "send_result").put("id", id).put("ok", ok)
        if (error != null) message.put("error", error)
        try {
            sendLocal(message)
        } catch (_: IOException) {
            // Rust disconnected; its command/result state is authoritative after reconnect.
        }
    }

    @Throws(IOException::class)
    private fun sendLocal(message: JSONObject) {
        val bytes = (message.toString() + "\n").toByteArray(StandardCharsets.UTF_8)
        val socket = localSocket ?: throw IOException("local bridge unavailable")
        synchronized(localWriteLock) {
            socket.outputStream.write(bytes)
            socket.outputStream.flush()
        }
    }

    private fun requireBluetoothPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
            ContextCompat.checkSelfPermission(appContext, Manifest.permission.BLUETOOTH_CONNECT) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            throw SecurityException("Bluetooth permission denied")
        }
    }

    override fun close() {
        running.set(false)
        serverSocket?.closeQuietly()
        serverSocket = null
        val sockets = synchronized(sessionLock) {
            val all = sessions.values.toList() + inFlightDials.values.toList()
            sessions.clear()
            inFlightDials.clear()
            all
        }
        sockets.forEach { it.closeQuietly() }
        localSocket?.closeQuietly()
        localSocket = null
        localExecutor.shutdownNow()
        acceptExecutor.shutdownNow()
        dialExecutor.shutdownNow()
        ioExecutor.shutdownNow()
        writeExecutor.shutdownNow()
    }

    private fun readBoundedLine(input: InputStream, maxBytes: Int): String? {
        val out = ByteArrayOutputStream()
        while (true) {
            val value = input.read()
            if (value == -1) return if (out.size() == 0) null else throw EOFException()
            if (value == '\n'.code) return out.toString(StandardCharsets.UTF_8.name())
            if (out.size() >= maxBytes) throw IOException("local command too large")
            out.write(value)
        }
    }
}

private fun Closeable.closeQuietly() {
    try {
        close()
    } catch (_: Exception) {
    }
}
