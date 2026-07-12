import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:image/image.dart' as img;
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';
import 'package:zxing2/qrcode.dart';
import 'dart:ffi' as ffi;
import 'package:ffi/ffi.dart';

// ── theme definitions ───────────────────────────────────────────────────────

class ThemeDef {
  const ThemeDef({
    required this.name,
    required this.primary,
    required this.bg,
    required this.surface,
    required this.surface2,
    required this.border,
    required this.text,
    required this.textDim,
    required this.bubbleOut,
    required this.bubbleIn,
    required this.errorBg,
    required this.errorFg,
    required this.selectedTile,
  });

  final String name;
  final Color primary;
  final Color bg;
  final Color surface;
  final Color surface2;
  final Color border;
  final Color text;
  final Color textDim;
  final Color bubbleOut;
  final Color bubbleIn;
  final Color errorBg;
  final Color errorFg;
  final Color selectedTile;
}

const _themes = <String, ThemeDef>{
  'Teal': ThemeDef(
    name: 'Teal',
    primary: Color(0xFF26D9C8),
    bg: Color(0xFF0E1117),
    surface: Color(0xFF161B22),
    surface2: Color(0xFF1C2333),
    border: Color(0xFF21262D),
    text: Color(0xFFE6EDF3),
    textDim: Color(0xFF7D8590),
    bubbleOut: Color(0xFF0D2847),
    bubbleIn: Color(0xFF1C2128),
    errorBg: Color(0xFF3D0F0F),
    errorFg: Color(0xFFFF7B72),
    selectedTile: Color(0xFF0D1F2D),
  ),
  'Purple': ThemeDef(
    name: 'Purple',
    primary: Color(0xFFBC8CFF),
    bg: Color(0xFF0D0B12),
    surface: Color(0xFF161220),
    surface2: Color(0xFF1E1830),
    border: Color(0xFF2A2240),
    text: Color(0xFFE8E0F8),
    textDim: Color(0xFF7E70A0),
    bubbleOut: Color(0xFF1A0D30),
    bubbleIn: Color(0xFF1E1828),
    errorBg: Color(0xFF3D0F0F),
    errorFg: Color(0xFFFF7B72),
    selectedTile: Color(0xFF1A1030),
  ),
  'Orange': ThemeDef(
    name: 'Orange',
    primary: Color(0xFFFF8C42),
    bg: Color(0xFF100D0A),
    surface: Color(0xFF1A1510),
    surface2: Color(0xFF241C14),
    border: Color(0xFF2E2418),
    text: Color(0xFFF0E8DC),
    textDim: Color(0xFF908070),
    bubbleOut: Color(0xFF2A1508),
    bubbleIn: Color(0xFF1E1810),
    errorBg: Color(0xFF3D0F0F),
    errorFg: Color(0xFFFF7B72),
    selectedTile: Color(0xFF2A1A0C),
  ),
  'Rose': ThemeDef(
    name: 'Rose',
    primary: Color(0xFFFF6B9D),
    bg: Color(0xFF100A0E),
    surface: Color(0xFF1A1018),
    surface2: Color(0xFF241620),
    border: Color(0xFF2E1A28),
    text: Color(0xFFF0DCE8),
    textDim: Color(0xFF907088),
    bubbleOut: Color(0xFF2A0A18),
    bubbleIn: Color(0xFF1E1018),
    errorBg: Color(0xFF3D0F0F),
    errorFg: Color(0xFFFF7B72),
    selectedTile: Color(0xFF2A1020),
  ),
  'Blue': ThemeDef(
    name: 'Blue',
    primary: Color(0xFF58A6FF),
    bg: Color(0xFF0A0E14),
    surface: Color(0xFF101822),
    surface2: Color(0xFF162030),
    border: Color(0xFF1E2A38),
    text: Color(0xFFD8E4F0),
    textDim: Color(0xFF687890),
    bubbleOut: Color(0xFF081428),
    bubbleIn: Color(0xFF101820),
    errorBg: Color(0xFF3D0F0F),
    errorFg: Color(0xFFFF7B72),
    selectedTile: Color(0xFF0C1828),
  ),
  'Green': ThemeDef(
    name: 'Green',
    primary: Color(0xFF3FB950),
    bg: Color(0xFF0A100C),
    surface: Color(0xFF101A12),
    surface2: Color(0xFF16241A),
    border: Color(0xFF1E2E20),
    text: Color(0xFFDCF0E0),
    textDim: Color(0xFF709078),
    bubbleOut: Color(0xFF082010),
    bubbleIn: Color(0xFF101A12),
    errorBg: Color(0xFF3D0F0F),
    errorFg: Color(0xFFFF7B72),
    selectedTile: Color(0xFF0C1E10),
  ),
};

ThemeDef _themeDef(String name) => _themes[name] ?? _themes['Teal']!;

ThemeData _buildTheme(ThemeDef t) => ThemeData(
      useMaterial3: true,
      brightness: Brightness.dark,
      colorScheme: ColorScheme.dark(
        primary: t.primary,
        surface: t.surface,
        error: t.errorFg,
      ),
      scaffoldBackgroundColor: t.bg,
      dividerColor: t.border,
      hintColor: t.textDim,
      appBarTheme: AppBarTheme(
        backgroundColor: t.surface,
        elevation: 0,
        centerTitle: false,
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: t.surface2,
        border: _inputBorder(t.border),
        enabledBorder: _inputBorder(t.border),
        focusedBorder: _inputBorder(t.primary),
        hintStyle: TextStyle(color: t.textDim, fontSize: 14),
        contentPadding:
            const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: t.primary,
          foregroundColor: t.bg,
          padding: const EdgeInsets.all(14),
          shape:
              RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        ),
      ),
      listTileTheme: ListTileThemeData(
        selectedTileColor: t.selectedTile,
        selectedColor: t.primary,
        iconColor: t.textDim,
        textColor: t.text,
        dense: true,
        contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 4),
      ),
      iconButtonTheme: IconButtonThemeData(
        style: IconButton.styleFrom(foregroundColor: t.textDim),
      ),
      textTheme: TextTheme(
        titleLarge: TextStyle(
          color: t.text,
          fontSize: 15,
          fontWeight: FontWeight.w600,
          letterSpacing: 0.1,
          fontFamilyFallback: const ['SidebandEmoji'],
        ),
        bodyLarge: TextStyle(color: t.text, fontSize: 14, height: 1.4, fontFamilyFallback: const ['SidebandEmoji']),
        bodyMedium: TextStyle(color: t.text, fontSize: 13, height: 1.4, fontFamilyFallback: const ['SidebandEmoji']),
        bodySmall: TextStyle(color: t.textDim, fontSize: 11, fontFamilyFallback: const ['SidebandEmoji']),
      ),
    );

InputBorder _inputBorder(Color c) => OutlineInputBorder(
      borderRadius: BorderRadius.circular(12),
      borderSide: BorderSide(color: c),
    );

// ── window handler ──────────────────────────────────────────────────────────

class _WindowHandler extends WindowListener {
  _WindowHandler(this._state);
  final _ChatScreenState _state;

  @override
  void onWindowClose() {
    if (_state._minimizeToTrayEnabled) {
      unawaited(_state._minimizeToTray());
    } else {
      unawaited(windowManager.destroy());
    }
  }

  @override
  void onWindowMinimize() {
    if (_state._minimizeToTrayEnabled) {
      unawaited(_state._minimizeToTray());
    }
    // When not enabled, let the default minimize behavior happen
  }
}

bool get _isDesktop =>
    Platform.isLinux || Platform.isWindows || Platform.isMacOS;
bool get _canUseMobileBackend => Platform.isAndroid;

String decodeQrImage(Uint8List bytes) {
  final image = img.decodeImage(bytes);
  if (image == null) throw const FormatException('Unsupported image format');
  final pixels = image
      .convert(numChannels: 4)
      .getBytes(order: img.ChannelOrder.rgba)
      .buffer
      .asInt32List();
  final source = RGBLuminanceSource(image.width, image.height, pixels);
  return QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source))).text;
}

const _nativeChannel = MethodChannel('sideband/native');
// ── app ─────────────────────────────────────────────────────────────────────

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  if (Platform.isLinux || Platform.isWindows || Platform.isMacOS) {
    await windowManager.ensureInitialized();
    const options = WindowOptions(
      title: 'Sideband',
      size: Size(1280, 800),
      minimumSize: Size(760, 520),
      center: true,
      skipTaskbar: false,
    );
    windowManager.waitUntilReadyToShow(options, () async {
      await windowManager.show();
      await windowManager.focus();
      await windowManager.setSkipTaskbar(false);
    });
    await _initTray();
  }
  runApp(const SidebandApp());
}

Future<void> _initTray() async {
  try {
    await trayManager.setIcon('assets/icon_256x256.png');
    final menu = Menu(items: [
      MenuItem(key: 'show_window', label: 'Show Sideband'),
      MenuItem.separator(),
      MenuItem(key: 'exit_app', label: 'Exit'),
    ]);
    await trayManager.setContextMenu(menu);
  } catch (_) {
    // Tray is best-effort; some Wayland compositors don't support it.
  }
}

class SidebandApp extends StatefulWidget {
  const SidebandApp({super.key, this.skipListener = false});

  final bool skipListener;

  @override
  State<SidebandApp> createState() => _SidebandAppState();
}

class _SidebandAppState extends State<SidebandApp> {
  String _themeName = 'Teal';

  void _setTheme(String name) {
    setState(() => _themeName = name);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Sideband',
      theme: _buildTheme(_themeDef(_themeName)),
      home: _ChatScreen(
        onThemeChanged: _setTheme,
        skipListener: widget.skipListener,
      ),
      debugShowCheckedModeBanner: false,
    );
  }
}

// ── data ────────────────────────────────────────────────────────────────────

class Contact {
  const Contact({
    required this.name,
    required this.onion,
    required this.pubkey,
    required this.x25519Pubkey,
    required this.ratchetActive,
    this.pending = false,
    this.blocked = false,
  });
  final String name;
  final String onion;
  final String pubkey;
  final String x25519Pubkey;
  final bool ratchetActive;
  final bool pending;
  final bool blocked;

  String get initial => name.isNotEmpty ? name[0].toUpperCase() : '?';

  Color get avatarColor {
    final h = name.codeUnits.fold<int>(0, (a, b) => a + b) % 360;
    return HSLColor.fromAHSL(1, h.toDouble(), 0.45, 0.42).toColor();
  }

  String get shortOnion {
    if (onion.length <= 20) return onion;
    return '${onion.substring(0, 10)}…${onion.substring(onion.length - 8)}';
  }

  bool get staticKeyActive => x25519Pubkey.trim().isNotEmpty;

  String get securityIcon {
    if (ratchetActive) return '🔒';
    if (staticKeyActive) return '🔐';
    return '✍';
  }

  String get securityLabel {
    if (blocked) return 'Blocked';
    if (pending) return 'Pending approval';
    if (ratchetActive) return 'Double Ratchet';
    if (staticKeyActive) return 'Static key';
    return 'Signed only';
  }

  String get securityDescription {
    if (blocked) return 'Blocked: inbound messages are dropped.';
    if (pending) return 'Unknown verified sender. Add or block this contact.';
    if (ratchetActive) {
      return 'Double Ratchet active: encrypted with forward secrecy.';
    }
    if (staticKeyActive) {
      return 'Static X25519 encryption: encrypted, but no ratchet yet.';
    }
    return 'Signed-only legacy contact: no X25519 encryption key is present.';
  }
}

Contact? parseAddCommandContact(String raw) {
  final trimmed = raw.trim();
  if (!trimmed.startsWith('/add ')) return null;
  var parts = trimmed.split(RegExp(r'\s+'));
  // Recover a line whose two 44-char base64 keys got concatenated into one
  // 88-char token because a space was lost copying wrapped terminal output.
  if (parts.length == 4 && parts[3].length == 88) {
    parts = [
      parts[0],
      parts[1],
      parts[2],
      parts[3].substring(0, 44),
      parts[3].substring(44),
    ];
  }
  if (parts.length < 5) return null;
  return Contact(
    name: parts[1],
    onion: parts[2],
    pubkey: parts[3],
    x25519Pubkey: parts[4],
    ratchetActive: false,
    pending: false,
    blocked: false,
  );
}

class GroupInfo {
  const GroupInfo({
    required this.id,
    required this.title,
    required this.members,
  });

  final String id;
  final String title;
  final List<String> members;

  String get sidebarLabel => title.trim().isEmpty ? id : title;
  int get participantCount => members.length + 1;
  String get memberSummary =>
      participantCount == 1 ? '1 member' : '$participantCount members';
  String get details => '$sidebarLabel\nid=$id\nmembers=${members.join(', ')}';
}

bool shouldFallbackToGlobalHistory({
  required bool groupSelected,
  required bool filteredHistoryEmpty,
  required String? contact,
  required Iterable<String> knownContacts,
}) {
  if (groupSelected || !filteredHistoryEmpty) return false;
  final name = contact?.trim();
  if (name == null || name.isEmpty) return false;
  return !knownContacts.contains(name);
}

class ChatMsg {
  const ChatMsg({
    required this.id,
    required this.direction,
    required this.status,
    required this.contact,
    required this.group,
    required this.text,
    required this.tsMs,
  });

  final int id;
  final String direction;
  final String status;
  final String contact;
  final String group;
  final String text;
  final int tsMs;

  DateTime get ts => DateTime.fromMillisecondsSinceEpoch(tsMs);
  bool get out => direction == 'out';
  bool get failed => status == 'failed';
  bool get sending => status == 'sending';
  bool get delivered => status == 'delivered';
}

class _History {
  const _History({required this.msgs, required this.maxId, required this.bin});
  final List<ChatMsg> msgs;
  final int? maxId;
  final String bin;
}

class ParsedGroupPayload {
  const ParsedGroupPayload({
    required this.groupId,
    required this.title,
    required this.body,
  });

  final String groupId;
  final String title;
  final String body;
}

class AttachmentInfo {
  const AttachmentInfo(
      {required this.label, required this.path, required this.image});

  final String label;
  final String path;
  final bool image;
}

/// Parse the transfer hash/key out of a `sideband_api_list_transfers` line.
/// Outbound lines look like:
///   "outbound <hash> -> <contact> chunk <n>/<t> file=<name>"
/// Incoming lines look like:
///   "incoming <key> chunks <have>/<total>"
/// Returns the hash/key token, or null if the line is not recognized.
String? parseTransferHash(String line) {
  final trimmed = line.trim();
  final match =
      RegExp(r'^(?:outbound|incoming)\s+(\S+)').firstMatch(trimmed);
  if (match == null) return null;
  final hash = match.group(1);
  if (hash == null || hash.isEmpty) return null;
  return hash;
}

/// True for outbound transfer lines (resumable/cancelable via the hash).
bool isOutboundTransfer(String line) => line.trim().startsWith('outbound ');

/// Body text for a local message notification. Truncated to a reasonable
/// length so the notification stays a preview, not a transcript.
String notificationBody(String text, {int maxLen = 80}) {
  final trimmed = text.trim();
  if (trimmed.length <= maxLen) return trimmed;
  return '${trimmed.substring(0, maxLen)}…';
}

/// Stable per-conversation notification id so repeat messages from one sender
/// coalesce into a single notification. Kept within 31-bit positive range for
/// the Android notification manager.
int notificationIdForContact(String key) {
  return key.hashCode & 0x7fffffff;
}

/// True if [path] resolves to a file under the profile's `downloads/`
/// directory. Only received files land there, and the Kotlin `openFile` handler
/// rejects anything outside it, so we mirror that check Dart-side to avoid a
/// pointless platform round-trip and error for sent-file rows that point at
/// arbitrary picker paths.
///
/// [profilePath] is the value returned by the `profilePath` MethodChannel call,
/// which already ends in `.sideband` (`<filesDir>/.sideband`). The downloads
/// directory is therefore `<profilePath>/downloads`, matching the Kotlin
/// `<filesDir>/.sideband/downloads/` allow-root.
bool isUnderDownloadsDir(String path, String profilePath) {
  if (path.isEmpty || profilePath.isEmpty) return false;
  final downloads = _normalizeDirPath('$profilePath/downloads');
  final normalized = _normalizeDirPath(path);
  return normalized == downloads || normalized.startsWith('$downloads/');
}

String _normalizeDirPath(String path) {
  final segments = <String>[];
  for (final seg in path.split('/')) {
    if (seg.isEmpty || seg == '.') continue;
    if (seg == '..') {
      if (segments.isNotEmpty) segments.removeLast();
      continue;
    }
    segments.add(seg);
  }
  return '/${segments.join('/')}';
}

AttachmentInfo? parseAttachmentText(String text) {
  final trimmed = text.trim();
  final receivedMatch =
      RegExp(r'^\[file received: (.+)\]$').firstMatch(trimmed);
  if (receivedMatch != null && !trimmed.startsWith('[file received failed')) {
    final path = receivedMatch.group(1)!.trim();
    return AttachmentInfo(
      label: _basename(path),
      path: path,
      image: isImagePath(path),
    );
  }
  if (trimmed.startsWith('[file received failed') ||
      trimmed.startsWith('[file write failed') ||
      trimmed.startsWith('[file hash mismatch')) {
    return AttachmentInfo(label: trimmed, path: '', image: false);
  }

  final sent = RegExp(r'^\[file sent: (.+?) \((.+)\)\]$').firstMatch(trimmed);
  if (sent != null) {
    final pathOrName = sent.group(1)!.trim();
    return AttachmentInfo(
      label: _basename(pathOrName),
      path: pathOrName,
      image: isImagePath(pathOrName),
    );
  }

  final offer = RegExp(r'^\[file offer\] (.+?) \(\d+ bytes, \d+ chunks\)$')
      .firstMatch(trimmed);
  if (offer != null) {
    final name = offer.group(1)!.trim();
    return AttachmentInfo(label: name, path: '', image: isImagePath(name));
  }

  // File transfer errors — show as non-clickable attachment with error label
  final failed = RegExp(r'^\[file (?:received failed hash|write failed):')
      .firstMatch(trimmed);
  if (failed != null) {
    return AttachmentInfo(label: trimmed, path: '', image: false);
  }

  return null;
}

bool isImagePath(String path) {
  final lower = path.toLowerCase();
  return lower.endsWith('.png') ||
      lower.endsWith('.jpg') ||
      lower.endsWith('.jpeg') ||
      lower.endsWith('.gif') ||
      lower.endsWith('.webp') ||
      lower.endsWith('.bmp');
}

String _basename(String path) {
  final normalized = path.replaceAll('\\', '/');
  final idx = normalized.lastIndexOf('/');
  return idx >= 0 ? normalized.substring(idx + 1) : normalized;
}

ParsedGroupPayload? parseGroupPayloadText(String text) {
  Object? decoded;
  try {
    decoded = jsonDecode(text);
  } catch (_) {
    return null;
  }

  if (decoded is String) {
    try {
      decoded = jsonDecode(decoded);
    } catch (_) {
      return null;
    }
  }

  if (decoded is! Map) return null;
  if (decoded['kind'] != 'group_message') return null;
  final groupId = decoded['group_id']?.toString().trim() ?? '';
  final body = decoded['body']?.toString() ?? '';
  if (groupId.isEmpty) return null;
  return ParsedGroupPayload(
    groupId: groupId,
    title: decoded['group_title']?.toString() ?? '',
    body: body,
  );
}

ChatMsg normalizeRawGroupPayloadMessage(ChatMsg msg) {
  final payload = parseGroupPayloadText(msg.text);
  if (payload == null) return msg;
  return ChatMsg(
    id: msg.id,
    direction: msg.direction,
    status: msg.status,
    contact: msg.contact,
    group: payload.groupId,
    text: payload.body,
    tsMs: msg.tsMs,
  );
}

List<ChatMsg> visibleContactMessages(List<ChatMsg> msgs) =>
    msgs.where((m) => m.group.isEmpty).toList(growable: false);

List<ChatMsg> mergeRecoveredGroupMessages({
  required List<ChatMsg> groupRows,
  required List<ChatMsg> globalRows,
  required String groupId,
  required int limit,
}) {
  final byId = <int, ChatMsg>{};
  for (final msg in groupRows) {
    if (msg.group == groupId) byId[msg.id] = msg;
  }
  for (final msg in globalRows) {
    final normalized = normalizeRawGroupPayloadMessage(msg);
    if (normalized.group == groupId) byId[normalized.id] = normalized;
  }
  final merged = byId.values.toList();
  merged.sort((a, b) {
    final ts = b.tsMs.compareTo(a.tsMs);
    return ts != 0 ? ts : b.id.compareTo(a.id);
  });
  if (merged.length > limit) return merged.take(limit).toList(growable: false);
  return merged;
}

class ShareInfo {
  const ShareInfo({required this.command, required this.qr});
  final String command;
  final List<String> qr;
}

class _SendMessageIntent extends Intent {
  const _SendMessageIntent();
}

// ── cli ─────────────────────────────────────────────────────────────────────

List<String> groupCreateArgs({
  required String profile,
  required String title,
  required List<String> members,
}) {
  final args = [
    'group',
    'create',
    '--profile',
    profile,
    '--title',
    title.trim()
  ];
  for (final member
      in members.map((m) => m.trim()).where((m) => m.isNotEmpty)) {
    args.addAll(['--member', member]);
  }
  args.add('--json');
  return args;
}

List<String> groupDeleteArgs(
        {required String profile, required String group}) =>
    [
      'group',
      'delete',
      '--profile',
      profile,
      '--group',
      group.trim(),
    ];

List<String> groupRenameArgs({
  required String profile,
  required String group,
  required String title,
}) =>
    [
      'group',
      'rename',
      '--profile',
      profile,
      '--group',
      group.trim(),
      '--title',
      title.trim(),
      '--json',
    ];

List<String> groupMemberMutationArgs({
  required String profile,
  required String action,
  required String group,
  required String member,
}) =>
    [
      'group',
      action,
      '--profile',
      profile,
      '--group',
      group.trim(),
      '--member',
      member.trim(),
      '--json',
    ];

class _Cli {
  _Cli();

  static String _defaultBin() {
    final env = Platform.environment['SIDEBAND_BIN'];
    if (env != null && env.trim().isNotEmpty) return env;

    final exeDir = File(Platform.resolvedExecutable).parent.path;
    final candidates = [
      '$exeDir/sideband',
      '../target/debug/sideband',
      '../target/release/sideband',
      '../../target/debug/sideband',
      '../../target/release/sideband',
    ];
    for (final c in candidates) {
      if (File(c).existsSync()) return c;
    }
    return 'sideband';
  }

  final String _bin = _defaultBin();
  String get bin => _bin;
  final String profile =
      Platform.environment['SIDEBAND_PROFILE'] ?? '~/.sideband';

  String expandedProfilePath() {
    final p = profile;
    if (p == '~') return Platform.environment['HOME'] ?? p;
    if (p.startsWith('~/')) {
      final home = Platform.environment['HOME'];
      if (home != null && home.isNotEmpty) return '$home/${p.substring(2)}';
    }
    return p;
  }

  bool identityConfigured() =>
      File('${expandedProfilePath()}/identity.toml').existsSync();

  Future<String> initProfile(String name) =>
      _run(['init', '--profile', profile, '--name', name]);

  bool _ratchetActive(String contactName) {
    if (contactName.trim().isEmpty) return false;
    return File('${expandedProfilePath()}/ratchet/$contactName.bin')
        .existsSync();
  }

  Future<String> _run(List<String> args) async {
    final r = await Process.run(_bin, args);
    if (r.exitCode != 0) {
      final err = (r.stderr as String).trim();
      final out = (r.stdout as String).trim();
      final detail = err.isNotEmpty ? err : out;
      throw Exception(detail.isEmpty ? '$_bin exited ${r.exitCode}' : detail);
    }
    return (r.stdout as String).trim();
  }

  Future<String> identity() => _run(['identity', '--profile', profile]);

  Future<ShareInfo> share(String onion) async {
    final raw =
        await _run(['share', '--profile', profile, '--onion', onion, '--json']);
    final decoded = jsonDecode(raw);
    if (decoded is! Map) {
      throw Exception('share JSON was not an object');
    }
    final qr = decoded['qr'];
    if (qr is! List) {
      throw Exception('share QR JSON was not a list');
    }
    return ShareInfo(
      command: decoded['command'] as String,
      qr: qr.map((row) => row.toString()).toList(growable: false),
    );
  }

  Future<String> name([String? value]) {
    final args = ['name', '--profile', profile];
    if (value != null && value.trim().isNotEmpty) args.add(value.trim());
    return _run(args);
  }

  Future<void> addContact({
    required String name,
    required String onion,
    required String pubkey,
    required String x25519Pubkey,
  }) async {
    await _run([
      'contact',
      'add',
      '--profile',
      profile,
      '--name',
      name,
      '--onion',
      onion,
      '--pubkey',
      pubkey,
      '--x25519-pubkey',
      x25519Pubkey,
    ]);
  }

  Future<String> deleteContact(String name) => _run([
        'contact',
        'delete',
        '--profile',
        profile,
        '--name',
        name,
      ]);

  Future<String> acceptContact(String name) => _run([
        'contact',
        'accept',
        '--profile',
        profile,
        '--name',
        name,
      ]);

  Future<String> blockContact(String name) => _run([
        'contact',
        'block',
        '--profile',
        profile,
        '--name',
        name,
      ]);

  Future<String> unblockContact(String name) => _run([
        'contact',
        'unblock',
        '--profile',
        profile,
        '--name',
        name,
      ]);

  Future<String> deleteGroup(String group) =>
      _run(groupDeleteArgs(profile: profile, group: group));

  List<String> groupLeaveArgs(
          {required String profile, required String group}) =>
      [
        'group',
        'leave',
        '--profile',
        profile,
        '--group',
        group,
      ];

  Future<String> leaveGroup(String group) =>
      _run(groupLeaveArgs(profile: profile, group: group));

  Future<GroupInfo> renameGroup(
          {required String group, required String title}) =>
      _parseGroupFromArgs(
          groupRenameArgs(profile: profile, group: group, title: title),
          context: 'group rename');

  Future<GroupInfo> addGroupMember(
          {required String group, required String member}) =>
      _parseGroupFromArgs(
          groupMemberMutationArgs(
              profile: profile,
              action: 'member-add',
              group: group,
              member: member),
          context: 'group member add');

  Future<GroupInfo> removeGroupMember(
          {required String group, required String member}) =>
      _parseGroupFromArgs(
          groupMemberMutationArgs(
              profile: profile,
              action: 'member-remove',
              group: group,
              member: member),
          context: 'group member remove');

  Future<String> clearHistory({String? contact, String? group}) {
    final args = ['history', '--profile', profile, '--clear'];
    if (group != null && group.trim().isNotEmpty) {
      args.addAll(['--group', group.trim()]);
    } else if (contact != null && contact.trim().isNotEmpty) {
      args.addAll(['--contact', contact.trim()]);
    }
    return _run(args);
  }

  Future<String> ratchet(String contact) =>
      _run(['ratchet', '--profile', profile, contact]);

  Future<List<Contact>> contacts() async {
    final raw = await _run(['contact', 'list', '--profile', profile, '--json']);
    if (raw.isEmpty) return [];

    final decoded = jsonDecode(raw);
    if (decoded is! List) {
      throw Exception('contact list JSON was not a list');
    }

    final parsed = <Contact>[];
    for (final item in decoded) {
      if (item is! Map) continue;
      parsed.add(Contact(
        name: item['name'] as String,
        onion: item['onion'] as String,
        pubkey: item['pubkey_b64'] as String? ?? '',
        x25519Pubkey: item['x25519_pubkey_b64'] as String? ?? '',
        ratchetActive: _ratchetActive(item['name'] as String),
        pending: item['pending'] == true,
        blocked: item['blocked'] == true,
      ));
    }
    parsed.sort((a, b) => a.name.compareTo(b.name));
    return parsed;
  }

  Future<List<GroupInfo>> groups() async {
    final raw = await _run(['group', 'list', '--profile', profile, '--json']);
    if (raw.isEmpty) return [];

    final decoded = jsonDecode(raw);
    if (decoded is! List) {
      throw Exception('group list JSON was not a list');
    }

    final parsed = <GroupInfo>[];
    for (final item in decoded) {
      if (item is! Map) continue;
      final members = <String>[];
      final rawMembers = item['members'];
      if (rawMembers is List) {
        for (final member in rawMembers) {
          if (member is Map && member['contact'] != null) {
            members.add(member['contact'].toString());
          }
        }
      }
      parsed.add(GroupInfo(
        id: item['id'].toString(),
        title: item['title'].toString(),
        members: members,
      ));
    }
    parsed.sort((a, b) => a.sidebarLabel.compareTo(b.sidebarLabel));
    return parsed;
  }

  Future<GroupInfo> createGroup({
    required String title,
    required List<String> members,
  }) async {
    final raw = await _run(
        groupCreateArgs(profile: profile, title: title, members: members));
    return _parseGroup(raw, context: 'group create');
  }

  Future<GroupInfo> _parseGroupFromArgs(List<String> args,
      {required String context}) async {
    final raw = await _run(args);
    return _parseGroup(raw, context: context);
  }

  GroupInfo _parseGroup(String raw, {required String context}) {
    final decoded = jsonDecode(raw);
    if (decoded is! Map) {
      throw Exception('$context JSON was not an object');
    }
    final parsedMembers = <String>[];
    final rawMembers = decoded['members'];
    if (rawMembers is List) {
      for (final member in rawMembers) {
        if (member is Map && member['contact'] != null) {
          parsedMembers.add(member['contact'].toString());
        }
      }
    }
    return GroupInfo(
      id: decoded['id'].toString(),
      title: decoded['title'].toString(),
      members: parsedMembers,
    );
  }

  Future<_History> history(
      {String? contact, String? group, int limit = 80}) async {
    final args = [
      'history',
      '--profile',
      profile,
      '--limit',
      '$limit',
      '--json'
    ];
    if (group != null && group.trim().isNotEmpty) {
      args.addAll(['--group', group.trim()]);
    } else if (contact != null && contact.trim().isNotEmpty) {
      args.addAll(['--contact', contact.trim()]);
    }
    final raw = await _run(args);
    if (raw.isEmpty) {
      return _History(msgs: const [], maxId: null, bin: _bin);
    }

    final decoded = jsonDecode(raw);
    if (decoded is! List) {
      throw Exception('history JSON was not a list');
    }

    final parsed = <ChatMsg>[];
    for (final item in decoded) {
      if (item is! Map) continue;
      final conversationKind = (item['conversation_kind'] as String?) ?? '';
      final msg = ChatMsg(
        id: (item['id'] as num).toInt(),
        direction: item['direction'] as String,
        status: _statusLabel((item['status'] as num).toInt()),
        contact: item['contact'] as String,
        group: conversationKind == 'group'
            ? (item['conversation_id'] as String?) ?? ''
            : '',
        text: item['body'] as String,
        tsMs: (item['timestamp_ms'] as num).toInt(),
      );
      parsed.add(normalizeRawGroupPayloadMessage(msg));
    }
    parsed.sort((a, b) => b.id.compareTo(a.id));
    final maxId = parsed.isEmpty ? null : parsed.first.id;
    return _History(msgs: parsed, maxId: maxId, bin: _bin);
  }

  String _statusLabel(int status) {
    switch (status) {
      case 0:
        return 'sent';
      case 1:
        return 'delivered';
      case 2:
        return 'failed';
      default:
        return '?';
    }
  }

  Future<void> send({required String to, required String message}) async {
    await _run(
        ['send', '--profile', profile, '--to', to, '--message', message]);
  }
}

// Native/Dart signatures for the string-returning FFI entry points. Every
// argument and the return value is a `char*`, so the native and Dart forms are
// identical apart from the pointer element type being fixed to `Utf8`.
typedef _NativePtr1 = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>);
typedef _Ptr1 = ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>);
typedef _NativePtr2 = ffi.Pointer<Utf8> Function(
    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _Ptr2 = ffi.Pointer<Utf8> Function(
    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _NativePtr3 = ffi.Pointer<Utf8> Function(
    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);
typedef _Ptr3 = ffi.Pointer<Utf8> Function(
    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>);

class _MobileApi {
  _MobileApi()
      : _initProfile = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>)>('sideband_api_init_profile'),
        _status = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(
                ffi.Pointer<Utf8>)>('sideband_api_status'),
        _listContacts = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>)>('sideband_api_list_contacts'),
        _listGroups = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(
                ffi.Pointer<Utf8>)>('sideband_api_list_groups'),
        _listMessages = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.UintPtr),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>,
                    int)>('sideband_api_list_messages'),
        _listGroupMessages = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.UintPtr),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>,
                    int)>('sideband_api_list_group_messages'),
        _sendMessage = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(
                ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>)>('sideband_api_send_message'),
        _sendFile = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(
                ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>)>('sideband_api_send_file'),
        _addContact = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(
                ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>)>('sideband_api_add_contact'),
        _deleteContact = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_delete_contact'),
        _acceptContact = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_accept_contact'),
        _blockContact = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_block_contact'),
        _initRatchet = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_init_ratchet'),
        _unblockContact = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_unblock_contact'),
        _freeString = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Void Function(ffi.Pointer<Utf8>),
            void Function(ffi.Pointer<Utf8>)>('sideband_api_free_string'),
        _listenerStart = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>)>('sideband_api_listener_start'),
        _listenerStop = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<ffi.Pointer<Utf8> Function(),
                ffi.Pointer<Utf8> Function()>('sideband_api_listener_stop'),
        _createGroup = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(
                ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>)>('sideband_api_create_group'),
        _deleteGroup = ffi.DynamicLibrary.open('libsideband.so').lookupFunction<
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
            ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                ffi.Pointer<Utf8>)>('sideband_api_delete_group'),
        _clearHistory = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_clear_history'),
        _shareCommand = ffi.DynamicLibrary.open('libsideband.so')
            .lookupFunction<
                ffi.Pointer<Utf8> Function(
                    ffi.Pointer<Utf8>, ffi.Pointer<Utf8>),
                ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>,
                    ffi.Pointer<Utf8>)>('sideband_api_share_command');

  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _initProfile;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>) _status;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>) _listContacts;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>) _listGroups;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, int)
      _listMessages;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, int)
      _listGroupMessages;
  final ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>) _sendMessage;
  final ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>) _sendFile;
  final ffi.Pointer<Utf8> Function(
    ffi.Pointer<Utf8>,
    ffi.Pointer<Utf8>,
    ffi.Pointer<Utf8>,
    ffi.Pointer<Utf8>,
    ffi.Pointer<Utf8>,
  ) _addContact;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _deleteContact;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _acceptContact;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _blockContact;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _initRatchet;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _unblockContact;
  final void Function(ffi.Pointer<Utf8>) _freeString;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>) _listenerStart;
  final ffi.Pointer<Utf8> Function() _listenerStop;
  final ffi.Pointer<Utf8> Function(
      ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Pointer<Utf8>) _createGroup;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _deleteGroup;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _clearHistory;
  final ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>)
      _shareCommand;

  // New symbols (group messaging, group management, retry status, transfers)
  // are resolved lazily so that an older libsideband.so that predates them does
  // not hard-crash the app at startup. A missing symbol surfaces as a normal
  // Exception the first time the feature is actually used.
  static final ffi.DynamicLibrary _lib =
      ffi.DynamicLibrary.open('libsideband.so');

  final _lazy = <String, Object?>{};

  T _resolve<T extends Function>(String symbol, T Function() lookup) {
    final cached = _lazy[symbol];
    if (cached != null) return cached as T;
    try {
      final fn = lookup();
      _lazy[symbol] = fn;
      return fn;
    } catch (_) {
      throw Exception(
          'native backend is missing $symbol; rebuild libsideband.so '
          '(./build-android-rust.sh)');
    }
  }

  _Ptr3 _lookup3(String symbol) => _resolve<_Ptr3>(
      symbol, () => _lib.lookupFunction<_NativePtr3, _Ptr3>(symbol));

  _Ptr2 _lookup2(String symbol) => _resolve<_Ptr2>(
      symbol, () => _lib.lookupFunction<_NativePtr2, _Ptr2>(symbol));

  _Ptr1 _lookup1(String symbol) => _resolve<_Ptr1>(
      symbol, () => _lib.lookupFunction<_NativePtr1, _Ptr1>(symbol));

  _Ptr3 get _sendGroupMessage => _lookup3('sideband_api_send_group_message');
  _Ptr3 get _sendGroupFile => _lookup3('sideband_api_send_group_file');
  _Ptr3 get _renameGroup => _lookup3('sideband_api_rename_group');
  _Ptr3 get _groupAddMember => _lookup3('sideband_api_group_add_member');
  _Ptr3 get _groupRemoveMember => _lookup3('sideband_api_group_remove_member');
  _Ptr2 get _leaveGroup => _lookup2('sideband_api_leave_group');
  _Ptr1 get _retryStatus => _lookup1('sideband_api_retry_status');
  _Ptr1 get _listTransfers => _lookup1('sideband_api_list_transfers');
  _Ptr2 get _resumeTransfer => _lookup2('sideband_api_resume_transfer');
  _Ptr2 get _cancelTransfer => _lookup2('sideband_api_cancel_transfer');

  String? _profilePath;

  Future<String> profilePath() async {
    final cached = _profilePath;
    if (cached != null) return cached;
    final filesProfile =
        await _nativeChannel.invokeMethod<String>('profilePath');
    if (filesProfile == null || filesProfile.isEmpty) {
      throw Exception('Android profile path unavailable');
    }
    return _profilePath = filesProfile;
  }

  Future<bool> identityConfigured() async {
    return File('${await profilePath()}/identity.toml').exists();
  }

  T _decode<T>(ffi.Pointer<Utf8> ptr) {
    try {
      final decoded = jsonDecode(ptr.toDartString());
      if (decoded is! Map) throw Exception('native response was not an object');
      if (decoded['ok'] != true) {
        throw Exception(
            decoded['error']?.toString() ?? 'native backend failed');
      }
      return decoded['data'] as T;
    } finally {
      _freeString(ptr);
    }
  }

  R _withCString1<R>(
      String a, ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>) call) {
    final ca = a.toNativeUtf8();
    try {
      return _decode<R>(call(ca));
    } finally {
      calloc.free(ca);
    }
  }

  R _withCString2<R>(String a, String b,
      ffi.Pointer<Utf8> Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>) call) {
    final ca = a.toNativeUtf8();
    final cb = b.toNativeUtf8();
    try {
      return _decode<R>(call(ca, cb));
    } finally {
      calloc.free(ca);
      calloc.free(cb);
    }
  }

  Future<void> initProfile(String displayName) async {
    _withCString2<Object?>(await profilePath(), displayName, _initProfile);
  }

  Future<Map<String, dynamic>> status() async {
    return Map<String, dynamic>.from(
        _withCString1<Map<dynamic, dynamic>>(await profilePath(), _status));
  }

  Future<List<Contact>> contacts() async {
    final raw =
        _withCString1<List<dynamic>>(await profilePath(), _listContacts);
    final contacts = <Contact>[];
    for (final item in raw) {
      if (item is! Map) continue;
      contacts.add(Contact(
        name: item['name'] as String? ?? '',
        onion: item['onion'] as String? ?? '',
        pubkey: item['ed25519_pubkey_b64'] as String? ?? '',
        x25519Pubkey: item['x25519_pubkey_b64'] as String? ?? '',
        ratchetActive: item['ratchet_active'] == true,
        pending: item['pending'] == true,
        blocked: item['blocked'] == true,
      ));
    }
    contacts.sort((a, b) => a.name.compareTo(b.name));
    return contacts;
  }

  Future<List<GroupInfo>> groups() async {
    final raw = _withCString1<List<dynamic>>(await profilePath(), _listGroups);
    final groups = <GroupInfo>[];
    for (final item in raw) {
      if (item is! Map) continue;
      final members = <String>[];
      final rawMembers = item['members'];
      if (rawMembers is List) {
        for (final m in rawMembers) {
          if (m is String) members.add(m);
        }
      }
      groups.add(GroupInfo(
        id: item['id'] as String? ?? '',
        title: item['title'] as String? ?? '',
        members: members,
      ));
    }
    groups.sort((a, b) => a.sidebarLabel.compareTo(b.sidebarLabel));
    return groups;
  }

  Future<_History> history(
      {String? contact, String? group, int limit = 80}) async {
    final profile = (await profilePath()).toNativeUtf8();
    if (group != null && group.isNotEmpty) {
      final cgroup = group.toNativeUtf8();
      try {
        final raw =
            _decode<List<dynamic>>(_listGroupMessages(profile, cgroup, limit));
        final parsed = <ChatMsg>[];
        for (final item in raw) {
          if (item is! Map) continue;
          parsed.add(ChatMsg(
            id: (item['id'] as num).toInt(),
            direction: item['direction'] as String? ?? '',
            status: item['status'] as String? ?? '',
            contact: item['contact'] as String? ?? '',
            group: item['group_id'] as String? ?? '',
            text: item['body'] as String? ?? '',
            tsMs: (item['timestamp_ms'] as num?)?.toInt() ?? 0,
          ));
        }
        parsed.sort((a, b) => a.tsMs.compareTo(b.tsMs));
        final maxId = parsed.isEmpty
            ? null
            : parsed.map((m) => m.id).reduce((a, b) => a > b ? a : b);
        return _History(msgs: parsed, maxId: maxId, bin: 'libsideband.so');
      } finally {
        calloc.free(cgroup);
      }
    }
    final ccontact = (contact ?? '').toNativeUtf8();
    try {
      final raw =
          _decode<List<dynamic>>(_listMessages(profile, ccontact, limit));
      final parsed = <ChatMsg>[];
      for (final item in raw) {
        if (item is! Map) continue;
        parsed.add(ChatMsg(
          id: (item['id'] as num).toInt(),
          direction: item['direction'] as String? ?? '',
          status: item['status'] as String? ?? '',
          contact: item['contact'] as String? ?? '',
          group: '',
          text: item['body'] as String? ?? '',
          tsMs: (item['timestamp_ms'] as num?)?.toInt() ?? 0,
        ));
      }
      parsed.sort((a, b) => a.tsMs.compareTo(b.tsMs));
      final maxId = parsed.isEmpty
          ? null
          : parsed.map((m) => m.id).reduce((a, b) => a > b ? a : b);
      return _History(msgs: parsed, maxId: maxId, bin: 'libsideband.so');
    } finally {
      calloc.free(profile);
      calloc.free(ccontact);
    }
  }

  Future<void> send({required String to, required String message}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cto = to.toNativeUtf8();
    final cmessage = message.toNativeUtf8();
    try {
      _decode<Object?>(_sendMessage(profile, cto, cmessage));
    } finally {
      calloc.free(profile);
      calloc.free(cto);
      calloc.free(cmessage);
    }
  }

  Future<void> sendFile({required String to, required String path}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cto = to.toNativeUtf8();
    final cpath = path.toNativeUtf8();
    try {
      _decode<Object?>(_sendFile(profile, cto, cpath));
    } finally {
      calloc.free(profile);
      calloc.free(cto);
      calloc.free(cpath);
    }
  }

  Future<void> addContact({
    required String name,
    required String onion,
    required String pubkey,
    required String x25519Pubkey,
  }) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cname = name.toNativeUtf8();
    final conion = onion.toNativeUtf8();
    final cpubkey = pubkey.toNativeUtf8();
    final cx25519 = x25519Pubkey.toNativeUtf8();
    try {
      _decode<Object?>(_addContact(profile, cname, conion, cpubkey, cx25519));
    } finally {
      calloc.free(profile);
      calloc.free(cname);
      calloc.free(conion);
      calloc.free(cpubkey);
      calloc.free(cx25519);
    }
  }

  Future<void> deleteContact(String name) async {
    _withCString2<Object?>(await profilePath(), name, _deleteContact);
  }

  Future<void> acceptContact(String name) async {
    _withCString2<Object?>(await profilePath(), name, _acceptContact);
  }

  Future<void> blockContact(String name) async {
    _withCString2<Object?>(await profilePath(), name, _blockContact);
  }

  Future<void> ratchet(String contact) async {
    _withCString2<Object?>(await profilePath(), contact, _initRatchet);
  }

  Future<void> unblockContact(String name) async {
    _withCString2<Object?>(await profilePath(), name, _unblockContact);
  }

  Future<void> startListener() async {
    final profile = (await profilePath()).toNativeUtf8();
    try {
      _decode<Object?>(_listenerStart(profile));
    } finally {
      calloc.free(profile);
    }
  }

  void stopListener() {
    _decode<Object?>(_listenerStop());
  }

  Future<void> createGroup({
    required String title,
    required List<String> members,
  }) async {
    final profile = (await profilePath()).toNativeUtf8();
    final ctitle = title.toNativeUtf8();
    final cmembers = jsonEncode(members).toNativeUtf8();
    try {
      _decode<Object?>(_createGroup(profile, ctitle, cmembers));
    } finally {
      calloc.free(profile);
      calloc.free(ctitle);
      calloc.free(cmembers);
    }
  }

  Future<void> deleteGroup(String groupId) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    try {
      _decode<Object?>(_deleteGroup(profile, cgroup));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
    }
  }

  Future<void> sendGroupMessage(
      {required String groupId, required String message}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    final cmessage = message.toNativeUtf8();
    try {
      _decode<Object?>(_sendGroupMessage(profile, cgroup, cmessage));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
      calloc.free(cmessage);
    }
  }

  Future<void> sendGroupFile(
      {required String groupId, required String path}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    final cpath = path.toNativeUtf8();
    try {
      _decode<Object?>(_sendGroupFile(profile, cgroup, cpath));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
      calloc.free(cpath);
    }
  }

  GroupInfo _parseGroupInfo(Map<String, dynamic> raw) {
    final members = <String>[];
    final rawMembers = raw['members'];
    if (rawMembers is List) {
      for (final m in rawMembers) {
        if (m is String) members.add(m);
      }
    }
    return GroupInfo(
      id: raw['id'] as String? ?? '',
      title: raw['title'] as String? ?? '',
      members: members,
    );
  }

  Future<GroupInfo> renameGroup(
      {required String groupId, required String title}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    final ctitle = title.toNativeUtf8();
    try {
      return _parseGroupInfo(
          _decode<Map<String, dynamic>>(_renameGroup(profile, cgroup, ctitle)));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
      calloc.free(ctitle);
    }
  }

  Future<GroupInfo> addGroupMember(
      {required String groupId, required String member}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    final cmember = member.toNativeUtf8();
    try {
      return _parseGroupInfo(_decode<Map<String, dynamic>>(
          _groupAddMember(profile, cgroup, cmember)));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
      calloc.free(cmember);
    }
  }

  Future<GroupInfo> removeGroupMember(
      {required String groupId, required String member}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    final cmember = member.toNativeUtf8();
    try {
      return _parseGroupInfo(_decode<Map<String, dynamic>>(
          _groupRemoveMember(profile, cgroup, cmember)));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
      calloc.free(cmember);
    }
  }

  Future<GroupInfo> leaveGroup(String groupId) async {
    final profile = (await profilePath()).toNativeUtf8();
    final cgroup = groupId.toNativeUtf8();
    try {
      return _parseGroupInfo(
          _decode<Map<String, dynamic>>(_leaveGroup(profile, cgroup)));
    } finally {
      calloc.free(profile);
      calloc.free(cgroup);
    }
  }

  Future<int> retryStatus() async {
    final profile = (await profilePath()).toNativeUtf8();
    try {
      final raw = _decode<Map<dynamic, dynamic>>(_retryStatus(profile));
      return (raw['queued'] as num?)?.toInt() ?? 0;
    } finally {
      calloc.free(profile);
    }
  }

  Future<List<String>> listTransfers() async {
    final profile = (await profilePath()).toNativeUtf8();
    try {
      final raw = _decode<List<dynamic>>(_listTransfers(profile));
      return [for (final e in raw) e.toString()];
    } finally {
      calloc.free(profile);
    }
  }

  Future<bool> resumeTransfer(String hash) async {
    final profile = (await profilePath()).toNativeUtf8();
    final chash = hash.toNativeUtf8();
    try {
      return _decode<bool>(_resumeTransfer(profile, chash));
    } finally {
      calloc.free(profile);
      calloc.free(chash);
    }
  }

  Future<bool> cancelTransfer(String hash) async {
    final profile = (await profilePath()).toNativeUtf8();
    final chash = hash.toNativeUtf8();
    try {
      return _decode<bool>(_cancelTransfer(profile, chash));
    } finally {
      calloc.free(profile);
      calloc.free(chash);
    }
  }

  Future<void> clearHistory({String? contact}) async {
    final profile = (await profilePath()).toNativeUtf8();
    final ccontact = contact?.toNativeUtf8() ?? ffi.nullptr;
    try {
      _decode<Object?>(_clearHistory(profile, ccontact));
    } finally {
      calloc.free(profile);
      if (ccontact != ffi.nullptr) calloc.free(ccontact);
    }
  }

  Future<ShareInfo> share(String onion) async {
    final profile = (await profilePath()).toNativeUtf8();
    final conion = onion.toNativeUtf8();
    try {
      final raw = _decode<Map<String, dynamic>>(_shareCommand(profile, conion));
      final command = raw['command'] as String? ?? '';
      final qr = (raw['qr'] as List?)?.map((e) => e.toString()).toList() ?? [];
      return ShareInfo(command: command, qr: qr);
    } finally {
      calloc.free(profile);
      calloc.free(conion);
    }
  }
}

class _QrPainter extends CustomPainter {
  const _QrPainter(this.rows);

  final List<String> rows;

  @override
  void paint(Canvas canvas, Size size) {
    if (rows.isEmpty) return;
    final paint = Paint()..color = Colors.black;
    final n = rows.length;
    final cell = size.shortestSide / n;
    final offsetX = (size.width - cell * n) / 2;
    final offsetY = (size.height - cell * n) / 2;
    for (var y = 0; y < rows.length; y++) {
      final row = rows[y];
      for (var x = 0; x < row.length; x++) {
        if (row.codeUnitAt(x) == 49) {
          canvas.drawRect(
            Rect.fromLTWH(offsetX + x * cell, offsetY + y * cell, cell, cell),
            paint,
          );
        }
      }
    }
  }

  @override
  bool shouldRepaint(covariant _QrPainter oldDelegate) =>
      oldDelegate.rows != rows;
}

class _DisplayNameDialog extends StatefulWidget {
  const _DisplayNameDialog({required this.theme});

  final ThemeDef theme;

  @override
  State<_DisplayNameDialog> createState() => _DisplayNameDialogState();
}

class _DisplayNameDialogState extends State<_DisplayNameDialog> {
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _submit() {
    Navigator.pop(context, _controller.text.trim());
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.theme;
    return AlertDialog(
      backgroundColor: t.surface,
      title: Text('Set up Sideband', style: TextStyle(color: t.text)),
      content: TextField(
        controller: _controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Display name'),
        onSubmitted: (_) => _submit(),
      ),
      actions: [
        FilledButton(
          onPressed: _submit,
          child: const Text('Create profile'),
        ),
      ],
    );
  }
}

// ── screen ──────────────────────────────────────────────────────────────────

class _ChatScreen extends StatefulWidget {
  const _ChatScreen({
    required this.onThemeChanged,
    required this.skipListener,
  });
  final void Function(String) onThemeChanged;
  final bool skipListener;

  @override
  State<_ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<_ChatScreen>
    with TrayListener, WidgetsBindingObserver {
  final _cli = _Cli();
  _MobileApi? _mobile;
  bool _mobileApiAvailable = false;
  String? _mobileOnion;
  final _input = TextEditingController();
  final _scroll = ScrollController();
  final scaffoldKey = GlobalKey<ScaffoldState>();

  List<Contact> _contacts = [];
  List<GroupInfo> _groups = [];
  final _unreadContacts = <String>{};
  final _unreadGroups = <String>{};
  final _refreshSeenIds = <int>{};
  final _notifiedMessageIds = <int>{};
  String? _notificationText;
  Timer? _notificationTimer;
  bool _showInAppNotifications = true;
  bool _showSystemNotifications = true;
  bool _showAudibleNotifications = true;
  bool _minimizeToTrayEnabled = false;
  List<ChatMsg> _msgs = [];
  final List<ChatMsg> _pendingMsgs = [];
  Contact? _sel;
  GroupInfo? _selGroup;
  Process? _listener;
  bool _listenerRunning = false;
  String _listenerStatus = 'listener stopped';
  String? _listenerOnion;
  String _listenerLogTail = '';
  late final File _listenerLogFile;
  bool _loading = true;
  bool _sending = false;
  String? _error;
  DateTime? _lastSendStartedAt;
  Timer? _poll;
  String _selectedTheme = 'Teal';
  String? _mobileProfilePath;
  String? _pendingAttachmentPath;
  String? _pendingAttachmentName;
  int _pendingAttachmentSize = 0;

  // presence + typing + activity
  final Map<String, DateTime> _lastSeen = {};
  final Map<String, bool> _online = {};
  final Map<String, ChatMsg> _lastMsg = {};

  // retry queue
  int _retryQueued = 0;

  // app lifecycle (Android): drives foreground-service + notification behavior
  AppLifecycleState _lifecycleState = AppLifecycleState.resumed;
  bool _foregroundServiceRunning = false;
  bool _notificationPermissionRequested = false;
  bool get _appResumed => _lifecycleState == AppLifecycleState.resumed;

  String _formatBytes(int bytes) {
    if (bytes < 1024) return '${bytes}B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)}KB';
    return '${(bytes / (1024 * 1024)).toStringAsFixed(1)}MB';
  }

  // ── activity tracking ─────────────────────────────────────────────────────

  void _recordActivity(List<ChatMsg> msgs) {
    for (final m in msgs) {
      if (m.direction == 'in' && m.contact.isNotEmpty) {
        _lastSeen[m.contact] = m.ts;
        _online[m.contact] = true;
        _lastMsg[m.contact] = m;
      } else if (m.direction == 'out' && m.contact.isNotEmpty) {
        _lastMsg[m.contact] = m;
      }
    }
  }

  String _presenceLabel(String contactName) {
    final isOnline = _online[contactName] == true;
    final lastSeen = _lastSeen[contactName];
    if (isOnline) return 'online';
    if (lastSeen == null) return 'last seen unknown';
    final diff = DateTime.now().difference(lastSeen);
    if (diff.inMinutes < 1) return 'last seen just now';
    if (diff.inHours < 1) return 'last seen ${diff.inMinutes}m ago';
    if (diff.inDays < 1) return 'last seen ${diff.inHours}h ago';
    if (diff.inDays < 7) return 'last seen ${diff.inDays}d ago';
    return 'last seen ${lastSeen.month}/${lastSeen.day}';
  }

  Widget _presenceDot(String contactName) {
    final isOnline = _online[contactName] == true;
    final color = isOnline ? const Color(0xFF3FB950) : _t.textDim;
    return Container(
      width: 8,
      height: 8,
      margin: const EdgeInsets.only(right: 4),
      decoration: BoxDecoration(color: color, shape: BoxShape.circle),
    );
  }

  String _previewText(ChatMsg m) {
    if (m.out) {
      final prefix = m.sending ? '⏳ ' : m.failed ? '⚠ ' : '✓ ';
      return '$prefix${m.text}';
    }
    return m.text;
  }

  ThemeDef get _t => _themeDef(_selectedTheme);
  late _WindowHandler _windowHandler;

  @override
  void initState() {
    super.initState();
    trayManager.addListener(this);
    WidgetsBinding.instance.addObserver(this);
    _listenerLogFile = File('${_cli.expandedProfilePath()}/gui-listener.log');
    _initWindowListeners();
    WidgetsBinding.instance
        .addPostFrameCallback((_) => unawaited(_bootstrap()));
  }

  void _initWindowListeners() {
    _windowHandler = _WindowHandler(this);
    if (!_isDesktop) return;
    windowManager.setPreventClose(true);
    windowManager.addListener(_windowHandler);
  }

  Future<void> _bootstrap() async {
    try {
      if (_canUseMobileBackend) {
        await _bootstrapMobile();
        return;
      }
      if (!_cli.identityConfigured()) {
        final name = await _promptDisplayName();
        if (name == null || name.trim().isEmpty) {
          if (mounted) {
            setState(() {
              _loading = false;
              _error = 'Profile setup cancelled';
            });
          }
          return;
        }
        await _cli.initProfile(name.trim());
      }
      if (!widget.skipListener) await _startListener();
      await _load();
      _poll ??= Timer.periodic(const Duration(seconds: 6), (_) {
        _refresh();
        _queryRetryStatus();
      });
    } catch (e) {
      if (mounted) {
        setState(() {
          _loading = false;
          _error = '$e';
        });
      }
    }
  }

  bool _ensureMobileApi() {
    if (_mobile == null && !_mobileApiAvailable) {
      try {
        _mobile = _MobileApi();
        _mobileApiAvailable = true;
      } catch (e) {
        _mobileApiAvailable = false;
        if (mounted) {
          setState(() {
            _loading = false;
            _error =
                'Android native backend unavailable: libsideband.so not found. Build with ./build-android-rust.sh';
          });
        }
        return false;
      }
    }
    return _mobile != null;
  }

  Future<void> _bootstrapMobile() async {
    if (!_ensureMobileApi()) return;
    final mobile = _mobile;
    if (mobile == null) throw Exception('Android backend unavailable');
    _mobileProfilePath = await mobile.profilePath();
    if (!await mobile.identityConfigured()) {
      final name = await _promptDisplayName();
      if (name == null || name.trim().isEmpty) {
        if (mounted) {
          setState(() {
            _loading = false;
            _error = 'Profile setup cancelled';
          });
        }
        return;
      }
      await mobile.initProfile(name.trim());
      if (mounted) {
        setState(() {
          _error = null;
          _listenerStatus = 'profile created';
        });
      }
    }
    if (!await _loadMobile()) return;
    if (!widget.skipListener) await _startMobileListener();
    _poll ??= Timer.periodic(const Duration(seconds: 6), (_) {
      _refresh();
      _queryRetryStatus();
    });
  }

  Future<bool> _loadMobile() async {
    if (!_ensureMobileApi()) return false;
    final mobile = _mobile;
    if (mobile == null) throw Exception('Android backend unavailable');
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final c = await mobile.contacts();
      final g = await mobile.groups();
      Contact? selected = _sel;
      GroupInfo? selectedGroup = _selGroup;
      if (selectedGroup != null) {
        final idx = g.indexWhere((x) => x.id == selectedGroup!.id);
        selectedGroup = idx >= 0 ? g[idx] : null;
      }
      if (selected == null && c.isNotEmpty && selectedGroup == null) {
        selected = c.first;
      } else if (selected != null) {
        final idx = c.indexWhere((x) => x.name == selected!.name);
        selected = idx >= 0 ? c[idx] : (c.isNotEmpty ? c.first : null);
      }
      if (!mounted) return false;
      final history = selectedGroup != null
          ? await mobile.history(group: selectedGroup.id)
          : (selected == null
              ? const _History(msgs: [], maxId: null, bin: 'libsideband.so')
              : await mobile.history(contact: selected.name));
      if (!mounted) return false;
      setState(() {
        _contacts = c;
        _groups = g;
        _sel = selected;
        _selGroup = selectedGroup;
        _msgs = _mergePending(history.msgs);
        _listenerRunning = false;
        _listenerStatus = 'mobile backend ready';
        _loading = false;
      });
      return true;
    } catch (e) {
      if (!mounted) return false;
      setState(() {
        _error = '$e';
        _listenerRunning = false;
        _listenerStatus = 'mobile backend failed';
        _loading = false;
      });
      return false;
    }
  }

  Future<void> _startMobileListener() async {
    final mobile = _mobile;
    if (mobile == null) {
      _snack('Android backend unavailable');
      return;
    }
    try {
      await mobile.startListener();
      await _syncMobileListenerStatus();
      if (mounted) {
        setState(() {
          _listenerRunning = true;
        });
      }
      // Keep the Rust listener alive while backgrounded, and ask for the
      // notification permission the first time (non-blocking — do not gate the
      // listener on the result).
      unawaited(_startForegroundService());
      unawaited(_requestNotificationPermission());
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _listenerStatus = 'mobile listener failed';
        _error = '$e';
      });
    }
  }

  Future<void> _syncMobileListenerStatus() async {
    final mobile = _mobile;
    if (mobile == null || !mounted) return;
    final status = await mobile.status();
    if (!mounted) return;
    setState(() {
      _mobileProfilePath = status['profile'] as String? ?? _mobileProfilePath;
      _listenerStatus = status['listener_status'] as String? ?? _listenerStatus;
      final onion = status['listener_onion'] as String? ?? '';
      if (onion.isNotEmpty) {
        _mobileOnion = onion;
      }
      _listenerRunning =
          status['listener_running'] as bool? ?? _listenerRunning;
      if (_listenerRunning) {
        _error = null;
      }
    });
  }

  Future<String?> _promptDisplayName() async {
    return showDialog<String>(
      context: context,
      barrierDismissible: false,
      builder: (_) => _DisplayNameDialog(theme: _t),
    );
  }

  @override
  void dispose() {
    trayManager.removeListener(this);
    WidgetsBinding.instance.removeObserver(this);
    windowManager.removeListener(_windowHandler);
    _poll?.cancel();
    _notificationTimer?.cancel();
    _listener?.kill(ProcessSignal.sigterm);
    unawaited(_stopForegroundService());
    _input.dispose();
    _scroll.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    super.didChangeAppLifecycleState(state);
    final wasResumed = _appResumed;
    _lifecycleState = state;
    if (!Platform.isAndroid) return;
    if (state == AppLifecycleState.resumed && !wasResumed) {
      // Coming back to the foreground: clear any message notifications the user
      // has now seen.
      unawaited(_cancelMessageNotifications());
    }
  }

  // ── Android foreground service + notifications ────────────────────────────

  Future<void> _startForegroundService() async {
    if (!Platform.isAndroid || _foregroundServiceRunning) return;
    try {
      await _nativeChannel.invokeMethod<void>('startForegroundService');
      _foregroundServiceRunning = true;
    } catch (_) {
      // MissingPluginException on desktop, or a platform failure — non-fatal.
    }
  }

  Future<void> _stopForegroundService() async {
    if (!Platform.isAndroid || !_foregroundServiceRunning) return;
    try {
      await _nativeChannel.invokeMethod<void>('stopForegroundService');
    } catch (_) {
      // best-effort; the OS reclaims the service when the process dies anyway.
    }
    _foregroundServiceRunning = false;
  }

  Future<void> _requestNotificationPermission() async {
    if (!Platform.isAndroid || _notificationPermissionRequested) return;
    _notificationPermissionRequested = true;
    try {
      await _nativeChannel.invokeMethod<bool>('requestNotificationPermission');
    } catch (_) {
      // request_in_progress or desktop MissingPluginException — non-fatal.
    }
  }

  Future<void> _cancelMessageNotifications() async {
    if (!Platform.isAndroid) return;
    try {
      await _nativeChannel.invokeMethod<void>('cancelMessageNotifications');
    } catch (_) {}
  }

  Future<void> _showMessageNotification(
      {required String title, required String body, required int id}) async {
    if (!Platform.isAndroid) return;
    try {
      await _nativeChannel.invokeMethod<void>('showMessageNotification', {
        'title': title,
        'body': body,
        'id': id,
      });
    } catch (_) {}
  }

  @override
  void onTrayIconMouseDown() {
    trayManager.popUpContextMenu();
  }

  @override
  void onTrayMenuItemClick(MenuItem menuItem) {
    if (menuItem.key == 'show_window') {
      unawaited(_showWindow());
    } else if (menuItem.key == 'exit_app') {
      unawaited(_exitApp());
    }
  }

  Future<void> _exitApp() async {
    await windowManager.hide();
    await windowManager.setSkipTaskbar(true);
    exit(0);
  }

  Future<void> _minimizeToTray() async {
    if (!(Platform.isLinux || Platform.isWindows || Platform.isMacOS)) return;
    await windowManager.hide();
    await windowManager.setSkipTaskbar(true);
  }

  Future<void> _appendListenerLog(String stream, String chunk) async {
    final text = chunk.trim();
    if (text.isEmpty) return;
    final line = '[${DateTime.now().toIso8601String()}] $stream: $text\n';
    _listenerLogTail = (_listenerLogTail + line);
    if (_listenerLogTail.length > 4000) {
      _listenerLogTail =
          _listenerLogTail.substring(_listenerLogTail.length - 4000);
    }
    try {
      await _listenerLogFile.parent.create(recursive: true);
      await _listenerLogFile.writeAsString(line,
          mode: FileMode.append, flush: true);
    } catch (_) {
      // The visible UI error matters more than failing to write diagnostics.
    }
  }

  String _expandedProfilePath() {
    if (_canUseMobileBackend) {
      final p = _mobileProfilePath;
      if (p != null && p.isNotEmpty) return p;
    }
    return _cli.expandedProfilePath();
  }

  String _expandProfileArg(String p) {
    if (p == '~') return Platform.environment['HOME'] ?? p;
    if (p.startsWith('~/')) {
      final home = Platform.environment['HOME'];
      if (home != null && home.isNotEmpty) return '$home/${p.substring(2)}';
    }
    return p;
  }

  Future<int?> _findExistingListenerPid() async {
    if (!Platform.isLinux) return null;

    final profile = _expandedProfilePath();
    final proc = Directory('/proc');
    if (!await proc.exists()) return null;

    await for (final entry in proc.list(followLinks: false)) {
      final segments = entry.uri.pathSegments;
      final name = segments.length >= 2 ? segments[segments.length - 2] : '';
      final pid = int.tryParse(name);
      if (pid == null) continue;

      try {
        final raw = await File('/proc/$pid/cmdline').readAsBytes();
        if (raw.isEmpty) continue;
        final args = utf8
            .decode(raw)
            .split('\u0000')
            .where((s) => s.isNotEmpty)
            .toList();
        final profileArg = args.indexOf('--profile');
        final sameProfile = profileArg >= 0 &&
            profileArg + 1 < args.length &&
            _expandProfileArg(args[profileArg + 1]) == profile;
        final looksSideband =
            args.any((a) => a.endsWith('/sideband') || a == 'sideband');
        if (args.contains('serve') && sameProfile && looksSideband) return pid;
      } catch (_) {
        // Processes exit while /proc is being scanned. Expected.
      }
    }
    return null;
  }

  bool _isRecentSendTransient(String lower) {
    final last = _lastSendStartedAt;
    if (last == null) return false;
    final recent = DateTime.now().difference(last) < const Duration(seconds: 8);
    return recent &&
        (lower.contains('send error') ||
            lower.contains('resolve error') ||
            lower.contains('control error') ||
            lower.contains('stream not connected') ||
            lower.contains('end cell with reason misc'));
  }

  bool _isFatalBackendLine(String lower) {
    return lower.contains('fatal') ||
        lower.contains('panic') ||
        lower.contains('failed to bootstrap arti tor client') ||
        lower.contains('incorrect permissions');
  }

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scroll.hasClients) return;
      _scroll.animateTo(
        _scroll.position.maxScrollExtent,
        duration: const Duration(milliseconds: 180),
        curve: Curves.easeOut,
      );
    });
  }

  Future<void> _startListener() async {
    if (_listener != null) return;
    setState(() {
      _listenerStatus = 'listener starting';
      _listenerOnion = null;
      _listenerRunning = false;
    });

    try {
      final existingPid = await _findExistingListenerPid();
      if (existingPid != null) {
        await _appendListenerLog(
            'stdout', 'stopping stale listener (pid $existingPid)');
        Process.killPid(existingPid, ProcessSignal.sigterm);
        await Future<void>.delayed(const Duration(milliseconds: 800));
      }

      final p = await Process.start(
        _cli.bin,
        ['serve', '--profile', _cli.profile],
        mode: ProcessStartMode.normal,
      );
      _listener = p;
      if (mounted) {
        setState(() {
          _listenerRunning = false;
          _listenerStatus = 'listener bootstrapping';
        });
      }

      p.stdout.transform(systemEncoding.decoder).listen((chunk) {
        unawaited(_appendListenerLog('stdout', chunk));
        final msg = chunk.trim();
        if (msg.isNotEmpty && mounted) {
          final lines = msg
              .split('\n')
              .map((line) => line.trim())
              .where((line) => line.isNotEmpty)
              .toList();
          final onionLine = lines.lastWhere(
            (line) => line.startsWith('onion='),
            orElse: () => '',
          );
          final last = lines.isEmpty ? msg : lines.last;
          final lower = msg.toLowerCase();
          setState(() {
            if (onionLine.isNotEmpty) {
              _listenerOnion = onionLine.substring('onion='.length);
              _listenerRunning = true;
              _listenerStatus = 'listening $_listenerOnion';
              _error = null;
            } else if (!_listenerRunning) {
              _listenerStatus = last;
            }
            // Parse structured JSON responses from Rust backend
            for (final line in lines) {
              if (line.startsWith('__sideband_resp__:')) {
                final jsonStr = line.substring('__sideband_resp__:'.length);
                try {
                  final decoded = jsonDecode(jsonStr) as Map<String, dynamic>;
                  final type = decoded['type'] as String?;
                  switch (type) {
                    case 'error':
                      _error = decoded['message'] as String? ?? 'error';
                      // Remove any optimistic pending messages on send error
                      final errCmd = decoded['cmd'] as String?;
                      if (errCmd == 'file' ||
                          errCmd == 'send' ||
                          errCmd == 'group_send') {
                        _pendingMsgs.removeWhere((m) => m.out && m.sending);
                      }
                      if (!_isRecentSendTransient(lower)) {
                        _listenerStatus = 'error: $_error';
                      }
                      break;
                    case 'sent':
                    case 'group_sent':
                    case 'file_sent':
                    case 'group_file_sent':
                    case 'left':
                    case 'deleted':
                    case 'ack':
                      _error = null;
                      break;
                    case 'retry_status':
                      final newCount = (decoded['queued'] as int?) ?? 0;
                      if (newCount != _retryQueued) {
                        setState(() => _retryQueued = newCount);
                      }
                      break;
                  }
                } catch (_) {
                  // Not valid JSON, ignore
                }
              }
            }
            // Legacy string matching fallback
            if (_error == null &&
                (lower.contains('send error') ||
                    lower.contains('resolve error') ||
                    lower.contains('control error')) &&
                !_isRecentSendTransient(lower)) {
              _error = msg;
            }
          });
          if (lower.contains('message received') ||
              lower.contains('incoming connection') ||
              lower.contains('message sent') ||
              lower.contains('file sent') ||
              lower.contains('send error') ||
              lower.contains('resolve error')) {
            unawaited(_refresh());
          }
          // Also refresh on structured group events
          for (final line in lines) {
            if (line.startsWith('__sideband_resp__:')) {
              try {
                final decoded =
                    jsonDecode(line.substring('__sideband_resp__:'.length))
                        as Map<String, dynamic>;
                final type = decoded['type'];
                if (type == 'sent' ||
                    type == 'file_sent' ||
                    type == 'group_sent' ||
                    type == 'group_file_sent' ||
                    type == 'left' ||
                    type == 'deleted') {
                  unawaited(_refresh());
                }
              } catch (_) {}
            }
          }
        }
      });
      p.stderr.transform(systemEncoding.decoder).listen((chunk) {
        unawaited(_appendListenerLog('stderr', chunk));
        final msg = chunk.trim();
        if (msg.isNotEmpty && mounted) {
          final lower = msg.toLowerCase();
          setState(() {
            if (!_listenerRunning ||
                lower.contains('error') ||
                lower.contains('failed')) {
              _listenerStatus = msg.split('\n').last.trim();
            }
            // Arti logs plenty of scary-but-normal stream churn to stderr.
            // Only surface backend errors that require the user to act.
            if (_isFatalBackendLine(lower)) {
              _error = msg;
            }
          });
          if (lower.contains('message received') ||
              lower.contains('incoming connection')) {
            unawaited(_refresh());
          }
        }
      });
      p.exitCode.then((code) {
        if (!mounted) return;
        setState(() {
          _listener = null;
          _listenerRunning = false;
          _listenerOnion = null;
          _listenerStatus = 'listener exited: $code';
          if (code != 0) {
            final tail = _listenerLogTail.trim();
            _error = tail.isEmpty
                ? 'sideband serve exited: $code'
                : 'sideband serve exited: $code\n$tail';
          }
        });
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _listener = null;
        _listenerRunning = false;
        _listenerOnion = null;
        _listenerStatus = 'listener failed';
        _error = '$e';
      });
    }
  }

  bool _matchesPending(ChatMsg pending, ChatMsg stored) {
    if (!stored.out || stored.contact != pending.contact) return false;
    if (stored.text == pending.text) return true;
    // file sends: the optimistic text is "[file sent: <path> (sending…)]"
    // while the real stored text is "[file sent: <path> (<size> bytes, inline)]"
    // match by the file path prefix
    if (pending.text.startsWith('[file sent: ') &&
        stored.text.startsWith('[file sent: ')) {
      final pendingPath =
          pending.text.substring('[file sent: '.length).split(' (').first;
      final storedPath =
          stored.text.substring('[file sent: '.length).split(' (').first;
      if (pendingPath == storedPath) return true;
      // The optimistic row carries the full picker path, but the stored row may
      // only keep the basename (or vice versa). Fall back to a basename match so
      // the "(sending…)" ghost row still clears.
      return _basename(pendingPath) == _basename(storedPath);
    }
    return false;
  }

  List<ChatMsg> _mergePending(List<ChatMsg> history) {
    _pendingMsgs.removeWhere(
        (pending) => history.any((stored) => _matchesPending(pending, stored)));
    final merged = [..._pendingMsgs, ...history];
    merged.sort((a, b) => a.tsMs.compareTo(b.tsMs));
    return merged;
  }

  Color _securityColor(Contact c) => c.blocked
      ? _t.errorFg
      : c.pending
          ? const Color(0xFFFFC857)
          : c.ratchetActive
              ? _t.primary
              : c.staticKeyActive
                  ? const Color(0xFF9CDCFE)
                  : _t.textDim;

  IconData _securityIcon(Contact c) => c.blocked
      ? Icons.block
      : c.pending
          ? Icons.person_add_disabled_outlined
          : c.ratchetActive
              ? Icons.lock_rounded
              : c.staticKeyActive
                  ? Icons.lock_outline
                  : Icons.edit_outlined;

  Future<void> _sendViaListener(
      {String? to, String? group, required String message}) async {
    if (_canUseMobileBackend && _mobile != null) {
      if (group != null) {
        await _mobile!.sendGroupMessage(groupId: group, message: message);
        return;
      }
      if (to == null || to.isEmpty) {
        throw Exception('missing contact for Android send');
      }
      await _mobile!.send(to: to, message: message);
      return;
    }
    final listener = _listener;
    if (listener == null) {
      throw Exception(
          'listener control channel is not available; restart the GUI');
    }
    listener.stdin.writeln(jsonEncode({
      'cmd': group == null ? 'send' : 'group_send',
      if (to != null) 'to': to,
      if (group != null) 'group': group,
      'message': message,
    }));
    await listener.stdin.flush();
  }

  Future<void> _sendFileViaListener(
      {String? to, String? group, required String path}) async {
    if (_canUseMobileBackend && _mobile != null) {
      if (group != null) {
        await _mobile!.sendGroupFile(groupId: group, path: path);
        return;
      }
      if (to == null || to.isEmpty) {
        throw Exception('missing contact for Android file send');
      }
      await _mobile!.sendFile(to: to, path: path);
      return;
    }
    final listener = _listener;
    if (listener == null) {
      throw Exception(
          'listener control channel is not available; restart the GUI');
    }
    listener.stdin.writeln(jsonEncode({
      'cmd': 'file',
      if (to != null) 'to': to,
      if (group != null) 'group': group,
      'path': path,
    }));
    await listener.stdin.flush();
  }

  Future<void> _showFileDialog() async {
    final target = _sel?.name ?? _selGroup?.title ?? '';
    if (target.isEmpty) {
      _snack('No contact or group selected');
      return;
    }
    if (!mounted) return;
    final path = await _pickFile(target);
    if (path == null || path.isEmpty) return;
    setState(() {
      _pendingAttachmentPath = path;
      _pendingAttachmentName = path.split('/').last;
    });
  }

  void _clearPendingAttachment() {
    setState(() {
      _pendingAttachmentPath = null;
      _pendingAttachmentName = null;
      _pendingAttachmentSize = 0;
    });
  }

  void _snack(String msg) {
    if (!mounted) return;
    setState(() => _notificationText = msg);
    _notificationTimer?.cancel();
    _notificationTimer = Timer(const Duration(seconds: 4), () {
      if (mounted) setState(() => _notificationText = null);
    });
  }

  Future<String?> _pickFile(String target) async {
    // Use Flutter's platform file-selector plugin. On Linux this routes through
    // the desktop's native chooser/portal instead of our own fake browser.
    try {
      final picked = await openFile(
        acceptedTypeGroups: const [
          XTypeGroup(label: 'All files'),
        ],
        confirmButtonText: 'Send',
      ).timeout(const Duration(minutes: 10));
      final path = picked?.path.trim() ?? '';
      if (path.isEmpty) return null;
      try {
        final size = File(path).lengthSync();
        if (size > 100 * 1024 * 1024) {
          _snack('File too large (max 100 MB)');
          return null;
        }
        setState(() {
          _pendingAttachmentSize = size;
        });
      } catch (_) {}
      return path;
    } on TimeoutException {
      _snack('File picker timed out');
      return null;
    } catch (e) {
      _snack('File picker failed: $e');
      return null;
    }
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final c = await _cli.contacts();
      final g = await _cli.groups();
      var s = _sel;
      var sg = _selGroup;
      if (sg != null) {
        final idx = g.indexWhere((x) => x.id == sg!.id);
        sg = idx >= 0 ? g[idx] : null;
      }
      if (sg == null) {
        if (s == null && c.isNotEmpty) {
          s = c.first;
        } else if (s != null) {
          final idx = c.indexWhere((x) => x.name == s!.name);
          s = idx >= 0 ? c[idx] : (c.isNotEmpty ? c.first : null);
        }
      } else {
        s = null;
      }
      final h = await _historyVisibleFor(s?.name,
          group: sg?.id, knownContacts: c.map((c) => c.name));
      final global = await _cli.history(limit: 200);
      // Startup bootstrap must mark the global transcript as already seen.
      // Seeding only the selected conversation makes old messages in other
      // chats look "new" on the first poll after launch. Do this here, once,
      // not inside _checkUnread(), or real new messages get swallowed.
      _seedSeenIds(global);
      _seedSeenIds(h);
      _recordActivity(global.msgs);
      _recordActivity(h.msgs);
      setState(() {
        _contacts = c;
        _groups = g;
        _sel = s;
        _selGroup = sg;
        _msgs = _mergePending(h.msgs);
        _loading = false;
      });
      _scrollToBottom();
    } catch (e) {
      setState(() {
        _error = '$e';
        _loading = false;
      });
    }
  }

  Future<_History> _historyVisibleFor(
    String? contact, {
    String? group,
    Iterable<String>? knownContacts,
  }) async {
    final filtered = await _cli.history(contact: contact, group: group);
    if (group != null) {
      final global = await _cli.history(limit: 200);
      final merged = mergeRecoveredGroupMessages(
        groupRows: filtered.msgs,
        globalRows: global.msgs,
        groupId: group,
        limit: 80,
      );
      return _History(
        msgs: merged,
        maxId: merged.isEmpty
            ? null
            : merged.map((m) => m.id).reduce((a, b) => a > b ? a : b),
        bin: filtered.bin,
      );
    }

    final contactMsgs = visibleContactMessages(filtered.msgs);
    final visibleFiltered = _History(
      msgs: contactMsgs,
      maxId: contactMsgs.isEmpty
          ? null
          : contactMsgs.map((m) => m.id).reduce((a, b) => a > b ? a : b),
      bin: filtered.bin,
    );

    if (!shouldFallbackToGlobalHistory(
      groupSelected: false,
      filteredHistoryEmpty: visibleFiltered.msgs.isEmpty,
      contact: contact,
      knownContacts: knownContacts ?? _contacts.map((c) => c.name),
    )) {
      return visibleFiltered;
    }

    // If inbound was stored under a raw pubkey/verified-peer because the local
    // contact record is stale, a strict contact filter hides the only evidence.
    // Fall back to the global transcript only for unknown/stale contacts. Known
    // contacts with no history must show an empty pane, not somebody else's mail.
    return _cli.history();
  }

  void _queryRetryStatus() {
    if (_canUseMobileBackend && _mobile != null) {
      // On Android the retry count comes back synchronously from the FFI rather
      // than via listener status events.
      unawaited(() async {
        try {
          final queued = await _mobile!.retryStatus();
          if (mounted && queued != _retryQueued) {
            setState(() => _retryQueued = queued);
          }
        } catch (_) {
          // best-effort; an old .so without the symbol just leaves the banner off
        }
      }());
      return;
    }
    final listener = _listener;
    if (listener == null) return;
    try {
      listener.stdin.writeln(jsonEncode({'cmd': 'retry_status'}));
      listener.stdin.flush();
    } catch (_) {}
  }

  Future<void> _retryFailedMessage(ChatMsg m) async {
    if (!m.failed || m.contact.isEmpty) return;
    try {
      await _sendViaListener(to: m.contact, message: m.text);
    } catch (e) {
      setState(() => _error = 'Retry failed: $e');
    }
  }

  Future<void> _refresh() async {
    try {
      final Future<List<Contact>> cFut;
      final Future<List<GroupInfo>> gFut;
      final mobile = _mobile;
      if (_canUseMobileBackend && mobile != null) {
        cFut = mobile.contacts();
        gFut = mobile.groups();
      } else {
        cFut = _cli.contacts();
        gFut = _cli.groups();
      }
      final c = await cFut;
      final g = await gFut;
      var s = _sel;
      var sg = _selGroup;
      if (sg != null) {
        final idx = g.indexWhere((x) => x.id == sg!.id);
        sg = idx >= 0 ? g[idx] : null;
      }
      if (sg == null && s != null) {
        final idx = c.indexWhere((x) => x.name == s!.name);
        s = idx >= 0 ? c[idx] : null;
      }
      _contacts = c;
      _groups = g;
      _sel = s;
      _selGroup = sg;

      if (s == null && sg == null) {
        await _checkUnread();
        if (_canUseMobileBackend && _mobile != null) {
          await _syncMobileListenerStatus();
        }
        if (mounted) setState(() {});
        return;
      }

      final h = _canUseMobileBackend && _mobile != null
          ? await _historyVisibleForMobile(s?.name,
              group: sg?.id, knownContacts: c.map((x) => x.name))
          : await _historyVisibleFor(s?.name,
              group: sg?.id, knownContacts: c.map((x) => x.name));
      await _checkUnread();
      _recordActivity(h.msgs);
      if (_canUseMobileBackend && _mobile != null) {
        await _syncMobileListenerStatus();
      }
      setState(() {
        _msgs = _mergePending(h.msgs);
      });
      _scrollToBottom();
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = '$e');
    }
  }

  /// Load visible history via the correct backend: FFI on Android, CLI on
  /// desktop. Callers must use this rather than `_historyVisibleFor` directly,
  /// which is desktop-only and throws on Android.
  Future<_History> _historyDispatch(
    String? contact, {
    String? group,
    Iterable<String>? knownContacts,
  }) {
    return _canUseMobileBackend && _mobile != null
        ? _historyVisibleForMobile(contact,
            group: group, knownContacts: knownContacts)
        : _historyVisibleFor(contact, group: group, knownContacts: knownContacts);
  }

  Future<_History> _historyVisibleForMobile(
    String? contact, {
    String? group,
    Iterable<String>? knownContacts,
  }) async {
    final mobile = _mobile;
    if (mobile == null) return const _History(msgs: [], maxId: null, bin: '');
    if (group != null && group.isNotEmpty) {
      return mobile.history(group: group);
    }
    if (contact != null && contact.isNotEmpty) {
      return mobile.history(contact: contact);
    }
    return mobile.history(limit: 80);
  }

  Future<void> _checkUnread() async {
    try {
      final global = _canUseMobileBackend && _mobile != null
          ? await _mobile!.history(limit: 200)
          : await _cli.history(limit: 200);
      final currentContact = _sel?.name;
      final currentGroup = _selGroup?.id;
      int newUnread = 0;
      final List<ChatMsg> notifyMsgs = [];
      for (final m in global.msgs) {
        if (m.direction != 'in') continue;
        if (_refreshSeenIds.contains(m.id)) continue;
        final belongsHere = (currentGroup != null && m.group == currentGroup) ||
            (currentGroup == null &&
                currentContact != null &&
                m.contact == currentContact &&
                m.group.isEmpty);
        if (belongsHere) {
          _refreshSeenIds.add(m.id);
          continue;
        }
        _refreshSeenIds.add(m.id);
        newUnread++;
        if (m.group.isNotEmpty) {
          _unreadGroups.add(m.group);
        } else if (m.contact.isNotEmpty) {
          _unreadContacts.add(m.contact);
        }
        if (!_notifiedMessageIds.contains(m.id)) {
          notifyMsgs.add(m);
        }
      }
      if (newUnread > 0 && mounted) setState(() {});
      if (notifyMsgs.isNotEmpty && mounted) {
        _showNotifications(notifyMsgs);
      }
    } catch (_) {
      // best-effort; never break the UI over unread accounting
    }
  }

  void _showNotifications(List<ChatMsg> msgs) {
    for (final m in msgs) {
      _notifiedMessageIds.add(m.id);
    }
    // Android: post OS notifications for inbound messages that arrive while the
    // app is not in the foreground. Coalesce per sender via a stable id.
    if (Platform.isAndroid && !_appResumed && _showSystemNotifications) {
      for (final m in msgs) {
        if (m.direction != 'in') continue;
        final sender = m.contact.isNotEmpty ? m.contact : 'Unknown';
        final groupName = m.group.isNotEmpty ? _groupNameForId(m.group) : '';
        final title = groupName.isNotEmpty ? '$sender • $groupName' : sender;
        unawaited(_showMessageNotification(
          title: title,
          body: notificationBody(m.text),
          id: notificationIdForContact(
              m.group.isNotEmpty ? m.group : m.contact),
        ));
      }
    }
    final String text;
    if (msgs.length == 1) {
      final m = msgs.first;
      final sender = m.contact.isNotEmpty ? m.contact : 'Unknown';
      final groupName = m.group.isNotEmpty ? _groupNameForId(m.group) : '';
      final prefix = groupName.isNotEmpty ? ' in $groupName' : '';
      final preview =
          m.text.length > 80 ? '${m.text.substring(0, 80)}…' : m.text;
      text = 'New message from $sender$prefix: $preview';
    } else {
      text = '${msgs.length} new messages';
    }
    if (_showInAppNotifications) {
      setState(() => _notificationText = text);
      _notificationTimer?.cancel();
      _notificationTimer = Timer(const Duration(seconds: 6), () {
        if (mounted) setState(() => _notificationText = null);
      });
    }
    if (_showSystemNotifications) {
      unawaited(_showSystemNotification(text));
    }
    if (_showAudibleNotifications) {
      unawaited(_playNotificationSound());
    }
  }

  Future<void> _playNotificationSound() async {
    if (!Platform.isLinux) return;
    // Try canberra-gtk-play first (lightweight, standard on GNOME/XFCE),
    // then paplay with a built-in fallback sound.
    try {
      await Process.run('canberra-gtk-play', ['--id', 'message-new-instant']);
      return;
    } catch (_) {}
    try {
      // Use a simple ALSA/PulseAudio beep via paplay and a temp WAV.
      // Generate a short sine beep if no default sound is available.
      await Process.run('paplay',
          ['/usr/share/sounds/freedesktop/stereo/message-new-instant.oga']);
    } catch (_) {
      // Sound is best-effort; never break the UI over a missing sound backend.
    }
  }

  Future<void> _showSystemNotification(String text) async {
    if (!Platform.isLinux) return;
    try {
      final args = <String>[
        '--app-name=Sideband',
        '--icon=${_notificationIconPath()}',
        'Sideband',
        text,
      ];
      await Process.run('notify-send', args);
    } catch (_) {
      // Desktop notifications are best-effort. Keep the in-app banner as fallback.
    }
  }

  String _notificationIconPath() {
    try {
      final exeDir = File(Platform.resolvedExecutable).parent.path;
      final bundled =
          File('$exeDir/data/flutter_assets/assets/icon_256x256.png');
      if (bundled.existsSync()) return bundled.path;
    } catch (_) {}
    final sourceTree = File('assets/icon_256x256.png');
    if (sourceTree.existsSync()) return sourceTree.path;
    return 'sideband_gui';
  }

  Future<void> _showWindow() async {
    if (!(Platform.isLinux || Platform.isWindows || Platform.isMacOS)) return;
    await windowManager.setSkipTaskbar(false);
    await windowManager.show();
    await windowManager.restore();
    await windowManager.focus();
  }

  Widget _notificationBanner() {
    if (_notificationText == null) return const SizedBox.shrink();
    return Material(
      color: const Color(0xFF2A3A4A),
      elevation: 4,
      child: InkWell(
        onTap: () => setState(() => _notificationText = null),
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          child: Row(
            children: [
              const Icon(Icons.mail_outline,
                  color: Color(0xFF26D9C8), size: 18),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  _notificationText!,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: Colors.white, fontSize: 13),
                ),
              ),
              const SizedBox(width: 8),
              const Icon(Icons.close, color: Colors.white54, size: 16),
            ],
          ),
        ),
      ),
    );
  }

  String _groupNameForId(String id) {
    for (final g in _groups) {
      if (g.id == id) return g.sidebarLabel;
    }
    return id;
  }

  Future<void> _send() async {
    final c = _sel;
    final g = _selGroup;
    final t = _input.text.trim();
    final attachPath = _pendingAttachmentPath;

    if (t.isEmpty && attachPath == null) return;

    if (t.startsWith('/') && attachPath == null) {
      await _runSlashCommand(t);
      return;
    }

    if (c == null && g == null) return;

    _input.clear();
    _clearPendingAttachment();

    // optimistic
    final now = DateTime.now();
    final optimistic = <ChatMsg>[];
    if (t.isNotEmpty) {
      optimistic.add(ChatMsg(
          id: -now.millisecondsSinceEpoch,
          direction: 'out',
          status: 'sending',
          contact: c?.name ?? 'You',
          group: g?.id ?? '',
          text: t,
          tsMs: now.millisecondsSinceEpoch));
    }
    if (attachPath != null) {
      // Always show the attachment as its own optimistic row, even when text is
      // sent with it. Otherwise text+file sends look like the file vanished.
      optimistic.add(ChatMsg(
          id: -now.millisecondsSinceEpoch - 1,
          direction: 'out',
          status: 'sending',
          contact: c?.name ?? 'You',
          group: g?.id ?? '',
          text: '[file sent: $attachPath (sending…)]',
          tsMs: now.millisecondsSinceEpoch + (t.isNotEmpty ? 1 : 0)));
    }
    if (optimistic.isNotEmpty) {
      setState(() {
        _sending = true;
        _lastSendStartedAt = now;
        _error = null;
        _pendingMsgs.addAll(optimistic);
        _msgs = _mergePending(_msgs.where((m) => !m.sending).toList());
      });
      _scrollToBottom();
    }

    try {
      // send message text first (if any)
      if (t.isNotEmpty) {
        if (g != null) {
          await _sendViaListener(group: g.id, message: t);
        } else {
          await _sendViaListener(to: c!.name, message: t);
        }
      }

      // then send attachment (if any)
      if (attachPath != null) {
        final isGroup = g != null;
        final targetName = c?.name ?? '';
        await _sendFileViaListener(
          to: isGroup ? null : targetName,
          group: isGroup ? g.id : null,
          path: attachPath,
        );
      }

      await _refresh();
      _scrollToBottom();
    } catch (e) {
      final optimisticIds = optimistic.map((m) => m.id).toSet();
      _pendingMsgs.removeWhere((m) => optimisticIds.contains(m.id));
      setState(() => _error = '$e');
      await _refresh();
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  void _showInfo(String title, String body) {
    showDialog<void>(
      context: context,
      builder: (_) => AlertDialog(
        backgroundColor: _t.surface,
        title: Text(title, style: TextStyle(color: _t.text)),
        content: SingleChildScrollView(
          child: SelectableText(body,
              style: TextStyle(color: _t.text, fontSize: 12, height: 1.35)),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Close'),
          ),
        ],
      ),
    );
  }

  Future<bool> _confirm(String title, String body) async {
    return await showDialog<bool>(
          context: context,
          builder: (_) => AlertDialog(
            backgroundColor: _t.surface,
            title: Text(title, style: TextStyle(color: _t.text)),
            content: Text(body, style: TextStyle(color: _t.textDim)),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context, false),
                child: const Text('Cancel'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(context, true),
                child: const Text('Do it'),
              ),
            ],
          ),
        ) ??
        false;
  }

  Future<void> _runSlashCommand(String raw) async {
    final parts = raw.substring(1).trim().split(RegExp(r'\s+'));
    final cmd = parts.isEmpty ? '' : parts.first.toLowerCase();
    final arg = raw.substring(1).trim().contains(' ')
        ? raw.substring(raw.indexOf(' ') + 1).trim()
        : '';

    try {
      switch (cmd) {
        case 'help':
          _showInfo('Slash commands',
              '/send <contact> <msg>\n/group-create <title> <member> [member...]\n/group-delete <id-or-title>\n/group-leave <id-or-title>\n/group-rename <id-or-title> <new-title>\n/group-add <id-or-title> <member>\n/group-remove <id-or-title> <member>\n/group <id-or-title> <msg>\n/file <filepath>\n/history [contact]\n/history-group <id-or-title>\n/contacts\n/groups\n/who — show members of selected group\n/add <name> <onion> <ed25519_pk> <x25519_pk>\n/delete <contact>\n/name [display-name]\n/whoami\n/share\n/onion\n/ratchet <contact>\n/status\n/clear\n/clearhistory [contact]\n/settings');
          return;
        case 'send':
          if (parts.length < 3) {
            throw Exception('usage: /send <contact> <message>');
          }
          final contact = parts[1];
          final msg = raw.split(RegExp(r'\s+')).skip(2).join(' ');
          if (_canUseMobileBackend && _mobile != null) {
            await _mobile!.send(to: contact, message: msg);
          } else {
            await _cli.send(to: contact, message: msg);
          }
          await _refresh();
          return;
        case 'group-create':
          if (parts.length < 3) {
            throw Exception(
                'usage: /group-create <title> <member> [member...]');
          }
          if (_canUseMobileBackend && _mobile != null) {
            await _mobile!.createGroup(
              title: parts[1],
              members: parts.skip(2).toList(),
            );
          } else {
            final group = await _cli.createGroup(
              title: parts[1],
              members: parts.skip(2).toList(),
            );
            _showInfo('Group created', group.details);
          }
          await _refresh();
          return;
        case 'group-delete':
          if (parts.length != 2) {
            throw Exception('usage: /group-delete <id-or-title>');
          }
          if (_canUseMobileBackend && _mobile != null) {
            await _mobile!.deleteGroup(parts[1]);
          } else {
            _showInfo('Group deleted', await _cli.deleteGroup(parts[1]));
          }
          await _refresh();
          if (_selGroup?.id == parts[1] || _selGroup?.title == parts[1]) {
            setState(() {
              _selGroup = null;
              _msgs = const [];
            });
          }
          return;
        case 'group-leave':
          if (parts.length != 2) {
            throw Exception('usage: /group-leave <id-or-title>');
          }
          if (_canUseMobileBackend && _mobile != null) {
            final left = await _mobile!.leaveGroup(parts[1]);
            _showInfo('Left group', left.details);
          } else {
            _showInfo('Left group', await _cli.leaveGroup(parts[1]));
          }
          await _load();
          if (_selGroup?.id == parts[1] || _selGroup?.title == parts[1]) {
            setState(() {
              _selGroup = null;
              _msgs = const [];
            });
          }
          return;
        case 'group-rename':
          if (parts.length < 3) {
            throw Exception('usage: /group-rename <id-or-title> <new-title>');
          }
          final newGroupTitle = raw.split(RegExp(r'\s+')).skip(2).join(' ');
          final group = _canUseMobileBackend && _mobile != null
              ? await _mobile!.renameGroup(groupId: parts[1], title: newGroupTitle)
              : await _cli.renameGroup(group: parts[1], title: newGroupTitle);
          await _load();
          _showInfo('Group renamed', group.details);
          return;
        case 'group-add':
          if (parts.length != 3) {
            throw Exception('usage: /group-add <id-or-title> <member>');
          }
          final added = _canUseMobileBackend && _mobile != null
              ? await _mobile!.addGroupMember(groupId: parts[1], member: parts[2])
              : await _cli.addGroupMember(group: parts[1], member: parts[2]);
          await _load();
          _showInfo('Member added', added.details);
          return;
        case 'group-remove':
          if (parts.length != 3) {
            throw Exception('usage: /group-remove <id-or-title> <member>');
          }
          final removed = _canUseMobileBackend && _mobile != null
              ? await _mobile!
                  .removeGroupMember(groupId: parts[1], member: parts[2])
              : await _cli.removeGroupMember(group: parts[1], member: parts[2]);
          await _load();
          _showInfo('Member removed', removed.details);
          return;
        case 'group':
          if (parts.length < 3) {
            throw Exception('usage: /group <id-or-title> <message>');
          }
          await _sendViaListener(
              group: parts[1],
              message: raw.split(RegExp(r'\s+')).skip(2).join(' '));
          await _refresh();
          return;
        case 'history':
          final contact = parts.length > 1 ? parts[1] : null;
          final h = _canUseMobileBackend && _mobile != null
              ? await _mobile!.history(contact: contact, limit: 200)
              : await _cli.history(contact: contact, limit: 200);
          final lines = h.msgs.reversed.map((m) {
            final arrow = m.direction == 'out' ? '→' : '←';
            return '${_hm(m.ts)} $arrow ${m.contact}: ${m.text}';
          });
          _showInfo('History${contact == null ? '' : ' for $contact'}',
              lines.isEmpty ? '(no messages)' : lines.join('\n'));
          return;
        case 'history-group':
          if (parts.length < 2) {
            throw Exception('usage: /history-group <id-or-title>');
          }
          await _loadGroupHistory(parts[1]);
          return;
        case 'contacts':
          await _refresh();
          _showInfo(
              'Contacts',
              _contacts.isEmpty
                  ? '(no contacts)'
                  : _contacts
                      .map((c) =>
                          '${c.name}\nsecurity=${c.securityLabel}\nonion=${c.onion}\npubkey=${c.pubkey}\nx25519=${c.x25519Pubkey}')
                      .join('\n\n'));
          return;
        case 'groups':
          await _refresh();
          _showInfo(
              'Groups',
              _groups.isEmpty
                  ? '(no groups)'
                  : _groups.map((g) => g.details).join('\n\n'));
          return;
        case 'who':
          final g = _selGroup;
          if (g == null) {
            _showInfo('Who',
                'No group selected — select a group from the sidebar first.');
            return;
          }
          await _load();
          final sel = _groups.firstWhere((x) => x.id == g.id, orElse: () => g);
          if (sel.members.isEmpty) {
            _showInfo('Who', 'No member info for this group yet.');
            return;
          }
          final lines = sel.members.map((name) {
            final c = _contacts.where((x) => x.name == name).firstOrNull;
            final onion = c?.onion ?? '?';
            final sec = c?.securityLabel ?? 'Unknown';
            return '$name\n  onion=$onion\n  security=$sec';
          }).join('\n\n');
          _showInfo("'${sel.title}' members (${sel.members.length + 1})",
              'You (self)\n\n$lines');
          return;
        case 'add':
          // Route through the shared parser so it recovers keys concatenated by
          // a lost space when pasting a wrapped /add line.
          final add = parseAddCommandContact(raw);
          if (add == null) {
            throw Exception(
                'usage: /add <name> <onion> <ed25519_pubkey_b64> <x25519_pubkey_b64>');
          }
          if (_canUseMobileBackend && _mobile != null) {
            await _mobile!.addContact(
              name: add.name,
              onion: add.onion,
              pubkey: add.pubkey,
              x25519Pubkey: add.x25519Pubkey,
            );
          } else {
            await _cli.addContact(
              name: add.name,
              onion: add.onion,
              pubkey: add.pubkey,
              x25519Pubkey: add.x25519Pubkey,
            );
          }
          final updatedContact = Contact(
            name: add.name,
            onion: add.onion,
            pubkey: add.pubkey,
            x25519Pubkey: add.x25519Pubkey,
            ratchetActive: false,
          );
          if (mounted) {
            setState(() {
              _contacts = [
                for (final c in _contacts)
                  if (c.name != updatedContact.name) c,
                updatedContact,
              ]..sort((a, b) => a.name.compareTo(b.name));
              _sel = updatedContact;
              _selGroup = null;
              _msgs = const [];
              _error = null;
            });
          }
          try {
            await _refresh();
          } catch (refreshError) {
            _snack('Contact saved; refresh failed: $refreshError');
          }
          return;
        case 'delete':
          if (parts.length < 2) throw Exception('usage: /delete <contact>');
          if (_canUseMobileBackend && _mobile != null) {
            await _mobile!.deleteContact(parts[1]);
          } else {
            _showInfo('Contact deleted', await _cli.deleteContact(parts[1]));
          }
          await _refresh();
          return;
        case 'name':
          _showInfo('Name', await _cli.name(arg.isEmpty ? null : arg));
          return;
        case 'whoami':
          _showInfo('Identity', await _cli.identity());
          return;
        case 'share':
          final onion = _currentOnion() ?? '(waiting for Tor)';
          if (_canUseMobileBackend && _mobile != null) {
            final share = await _mobile!.share(onion);
            _showInfo('Share contact', share.command);
          } else {
            final identity = await _cli.identity();
            String value(String prefix) {
              final line = identity
                  .split('\n')
                  .firstWhere((l) => l.startsWith(prefix), orElse: () => '');
              return line.isEmpty ? '' : line.substring(prefix.length).trim();
            }

            final name = value('name:');
            final ed = value('pubkey(ed25519,b64):');
            final x = value('pubkey(x25519,b64):');
            _showInfo('Share contact', '/add $name $onion $ed $x');
          }
          return;
        case 'onion':
          final onion = _currentOnion() ?? '(waiting for Tor)';
          _showInfo('Onion', onion);
          return;
        case 'ratchet':
          if (parts.length < 2) throw Exception('usage: /ratchet <contact>');
          if (_canUseMobileBackend && _mobile != null) {
            await _mobile!.ratchet(parts[1]);
          } else {
            await _cli.ratchet(parts[1]);
          }
          await _load();
          _showInfo('Ratchet',
              'Double Ratchet initialized for ${parts[1]}. Send a message to complete the handshake.');
          return;
        case 'status':
          _showInfo('Status',
              'listener: $_listenerStatus\nprofile: ${_runtimeProfileLabel()}\nbackend: ${_runtimeBackendLabel()}\ncontacts: ${_contacts.length}\ngroups: ${_groups.length}\nmessages visible: ${_msgs.length}');
          return;
        case 'clear':
          setState(() => _msgs = []);
          return;
        case 'clearhistory':
          final contact = parts.length > 1 ? parts[1] : _sel?.name;
          final clearTarget =
              contact == null ? 'all message history' : 'history for $contact';
          if (!await _confirm('Clear history', 'Delete $clearTarget?')) return;
          _showInfo(
              'History cleared', await _cli.clearHistory(contact: contact));
          await _load();
          return;
        case 'settings':
          await _showSettings();
          return;
        case 'file':
          if (parts.length < 2) {
            throw Exception('usage: /file <filepath> — or use the 📎 button');
          }
          final path = parts.skip(1).join(' ');
          final target = _sel?.name ?? _selGroup?.title;
          if (target == null) throw Exception('no contact or group selected');
          final isGroup = _selGroup != null;
          await _sendFileViaListener(
            to: isGroup ? null : target,
            group: isGroup ? target : null,
            path: path,
          );
          await _refresh();
          _scrollToBottom();
          return;
        case 'transfers':
          if (_canUseMobileBackend && _mobile != null) {
            await _showTransfersSheet();
            return;
          }
          throw Exception(
              'The transfers UI is available on Android and via the TUI. '
              'On desktop, manage transfers from the TUI for now.');
        default:
          throw Exception('unknown command: /$cmd (try /help)');
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _showTransfersSheet() async {
    final mobile = _mobile;
    if (mobile == null) {
      _snack('Android backend unavailable');
      return;
    }
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: _t.surface,
      isScrollControlled: true,
      builder: (sheetContext) {
        return StatefulBuilder(
          builder: (sheetContext, setSheetState) {
            Future<void> reload() async => setSheetState(() {});
            return SafeArea(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 16, 16, 12),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Icon(Icons.swap_vert, color: _t.primary, size: 20),
                        const SizedBox(width: 8),
                        Text('File transfers',
                            style: TextStyle(
                                color: _t.text,
                                fontSize: 16,
                                fontWeight: FontWeight.w700)),
                        const Spacer(),
                        IconButton(
                          icon: Icon(Icons.refresh, color: _t.textDim),
                          onPressed: reload,
                        ),
                      ],
                    ),
                    const SizedBox(height: 4),
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxHeight: 380),
                      child: FutureBuilder<List<String>>(
                        future: mobile.listTransfers(),
                        builder: (context, snapshot) {
                          if (snapshot.connectionState !=
                              ConnectionState.done) {
                            return const Padding(
                              padding: EdgeInsets.all(24),
                              child: Center(child: CircularProgressIndicator()),
                            );
                          }
                          if (snapshot.hasError) {
                            return Padding(
                              padding: const EdgeInsets.symmetric(vertical: 16),
                              child: Text('${snapshot.error}',
                                  style: TextStyle(color: _t.errorFg)),
                            );
                          }
                          final transfers = snapshot.data ?? const [];
                          if (transfers.isEmpty) {
                            return Padding(
                              padding: const EdgeInsets.symmetric(vertical: 24),
                              child: Text('No active transfers.',
                                  style: TextStyle(color: _t.textDim)),
                            );
                          }
                          return ListView.separated(
                            shrinkWrap: true,
                            itemCount: transfers.length,
                            separatorBuilder: (_, __) => Divider(
                                height: 12, color: _t.border),
                            itemBuilder: (context, i) {
                              final line = transfers[i];
                              final hash = parseTransferHash(line);
                              final outbound = isOutboundTransfer(line);
                              return Row(
                                crossAxisAlignment: CrossAxisAlignment.center,
                                children: [
                                  Expanded(
                                    child: Text(line,
                                        style: TextStyle(
                                            color: _t.text, fontSize: 12)),
                                  ),
                                  if (hash != null && outbound) ...[
                                    TextButton(
                                      onPressed: () async {
                                        try {
                                          final ok =
                                              await mobile.resumeTransfer(hash);
                                          _snack(ok
                                              ? 'Resuming transfer'
                                              : 'No persisted transfer to resume');
                                          await reload();
                                        } catch (e) {
                                          _snack('Resume failed: $e');
                                        }
                                      },
                                      child: const Text('Resume'),
                                    ),
                                    TextButton(
                                      onPressed: () async {
                                        try {
                                          final ok =
                                              await mobile.cancelTransfer(hash);
                                          _snack(ok
                                              ? 'Transfer cancelled'
                                              : 'Nothing to cancel');
                                          await reload();
                                        } catch (e) {
                                          _snack('Cancel failed: $e');
                                        }
                                      },
                                      child: Text('Cancel',
                                          style: TextStyle(color: _t.errorFg)),
                                    ),
                                  ],
                                ],
                              );
                            },
                          );
                        },
                      ),
                    ),
                    const SizedBox(height: 8),
                    Align(
                      alignment: Alignment.centerRight,
                      child: TextButton(
                        onPressed: () => Navigator.pop(sheetContext),
                        child: const Text('Close'),
                      ),
                    ),
                  ],
                ),
              ),
            );
          },
        );
      },
    );
  }

  Future<void> _showAddContactDialog() => _showContactDialog();

  Future<void> _addParsedContact(Contact contact) async {
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.addContact(
          name: contact.name,
          onion: contact.onion,
          pubkey: contact.pubkey,
          x25519Pubkey: contact.x25519Pubkey,
        );
      } else {
        await _cli.addContact(
          name: contact.name,
          onion: contact.onion,
          pubkey: contact.pubkey,
          x25519Pubkey: contact.x25519Pubkey,
        );
      }
      if (mounted) {
        setState(() {
          _contacts = [
            for (final c in _contacts)
              if (c.name != contact.name) c,
            contact,
          ]..sort((a, b) => a.name.compareTo(b.name));
          _sel = contact;
          _selGroup = null;
          _msgs = const [];
          _error = null;
        });
      }
      try {
        await _refresh();
      } catch (refreshError) {
        _snack('Contact saved; refresh failed: $refreshError');
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _scanContactQr() async {
    if (!_canUseMobileBackend) {
      _snack('QR scanning is only wired on Android right now');
      return;
    }
    final controller = MobileScannerController(
      cameraResolution: const Size(1280, 720),
    );
    var handled = false;
    String? scannerError;
    try {
      final raw = await showDialog<String>(
        context: context,
        builder: (dialogContext) => AlertDialog(
          backgroundColor: _t.surface,
          title: Text('Scan contact QR', style: TextStyle(color: _t.text)),
          content: SizedBox(
            width: 320,
            height: 360,
            child: ClipRRect(
              borderRadius: BorderRadius.circular(12),
              child: MobileScanner(
                controller: controller,
                onDetectError: (error, stackTrace) {
                  if (error is MobileScannerException) {
                    final details = error.errorDetails?.details?.toString();
                    scannerError = 'MobileScanner ${error.errorCode.name}: '
                        '${error.errorDetails?.message ?? 'no details'}'
                        '${details == null || details.isEmpty ? '' : '\n$details'}';
                  } else {
                    scannerError = error.toString();
                  }
                },
                errorBuilder: (context, error) {
                  final details = error.errorDetails?.details?.toString();
                  scannerError = 'MobileScanner ${error.errorCode.name}: '
                      '${error.errorDetails?.message ?? 'no details'}'
                      '${details == null || details.isEmpty ? '' : '\n$details'}';
                  return Container(
                    color: Colors.black,
                    padding: const EdgeInsets.all(16),
                    alignment: Alignment.center,
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(Icons.error_outline,
                            color: Colors.white, size: 36),
                        const SizedBox(height: 12),
                        Text(
                          'Camera failed to start',
                          style: TextStyle(
                              color: _t.text, fontWeight: FontWeight.w600),
                          textAlign: TextAlign.center,
                        ),
                        const SizedBox(height: 8),
                        Text(
                          error.errorCode.name,
                          style: TextStyle(color: _t.text, fontSize: 13),
                          textAlign: TextAlign.center,
                        ),
                        const SizedBox(height: 6),
                        Text(
                          error.errorDetails?.message ?? 'no details',
                          style: TextStyle(color: _t.textDim, fontSize: 12),
                          textAlign: TextAlign.center,
                        ),
                        if (details != null && details.isNotEmpty) ...[
                          const SizedBox(height: 8),
                          Expanded(
                            child: SingleChildScrollView(
                              child: SelectableText(
                                details,
                                style: TextStyle(
                                    color: _t.textDim, fontSize: 10),
                                textAlign: TextAlign.left,
                              ),
                            ),
                          ),
                        ],
                      ],
                    ),
                  );
                },
                onDetect: (capture) {
                  if (handled) return;
                  for (final barcode in capture.barcodes) {
                    final value = barcode.rawValue;
                    if (value == null || value.trim().isEmpty) continue;
                    handled = true;
                    Navigator.pop(dialogContext, value);
                    break;
                  }
                },
              ),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(dialogContext),
              child: const Text('Cancel'),
            ),
          ],
        ),
      );
      if (raw == null) {
        if (scannerError != null && scannerError!.isNotEmpty) {
          throw Exception(scannerError!);
        }
        return;
      }
      final parsed = parseAddCommandContact(raw);
      if (parsed == null) {
        throw Exception('QR did not contain a Sideband /add contact command');
      }
      await _addParsedContact(parsed);
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      await controller.dispose();
    }
  }

  Future<void> _uploadContactQr() async {
    try {
      final file = await openFile(
        acceptedTypeGroups: const [
          XTypeGroup(
            label: 'QR code images',
            extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'],
          ),
        ],
        confirmButtonText: 'Open QR code',
      );
      if (file == null) return;

      final raw = decodeQrImage(await file.readAsBytes());
      final parsed = parseAddCommandContact(raw);
      if (parsed == null) {
        throw Exception('QR did not contain a Sideband /add contact command');
      }
      await _addParsedContact(parsed);
    } catch (e) {
      if (mounted) setState(() => _error = 'Could not read QR image: $e');
    }
  }

  Future<void> _showEditContactDialog(Contact contact) =>
      _showContactDialog(contact: contact);

  Future<void> _showContactDialog({Contact? contact}) async {
    final editing = contact != null;
    final name = TextEditingController(text: contact?.name ?? '');
    final onion = TextEditingController(text: contact?.onion ?? '');
    final pubkey = TextEditingController(text: contact?.pubkey ?? '');
    final x25519 = TextEditingController(text: contact?.x25519Pubkey ?? '');
    try {
      final ok = await showDialog<bool>(
        context: context,
        builder: (_) => AlertDialog(
          backgroundColor: _t.surface,
          title: Text(editing ? 'Edit contact' : 'Add contact',
              style: TextStyle(color: _t.text)),
          content: SizedBox(
            width: 520,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                TextField(
                    controller: name,
                    decoration: const InputDecoration(labelText: 'Name')),
                const SizedBox(height: 8),
                TextField(
                    controller: onion,
                    decoration:
                        const InputDecoration(labelText: 'Onion address')),
                const SizedBox(height: 8),
                TextField(
                    controller: pubkey,
                    decoration:
                        const InputDecoration(labelText: 'Ed25519 pubkey')),
                const SizedBox(height: 8),
                TextField(
                    controller: x25519,
                    decoration:
                        const InputDecoration(labelText: 'X25519 pubkey')),
              ],
            ),
          ),
          actions: [
            TextButton(
                onPressed: () => Navigator.pop(context, false),
                child: const Text('Cancel')),
            FilledButton(
                onPressed: () => Navigator.pop(context, true),
                child: Text(editing ? 'Save' : 'Add')),
          ],
        ),
      );
      if (ok != true) return;

      final pasted = !editing ? parseAddCommandContact(name.text) : null;
      final newName = (pasted?.name ?? name.text).trim();
      final newOnion = (pasted?.onion ?? onion.text).trim();
      final newPubkey = (pasted?.pubkey ?? pubkey.text).trim();
      final newX25519 = (pasted?.x25519Pubkey ?? x25519.text).trim();
      if (newName.isEmpty) throw Exception('contact name is required');
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.addContact(
          name: newName,
          onion: newOnion,
          pubkey: newPubkey,
          x25519Pubkey: newX25519,
        );
        if (contact != null && contact.name != newName) {
          await _mobile!.deleteContact(contact.name);
        }
      } else {
        await _cli.addContact(
          name: newName,
          onion: newOnion,
          pubkey: newPubkey,
          x25519Pubkey: newX25519,
        );
        if (contact != null && contact.name != newName) {
          await _cli.deleteContact(contact.name);
        }
      }
      final updatedContact = Contact(
        name: newName,
        onion: newOnion,
        pubkey: newPubkey,
        x25519Pubkey: newX25519,
        ratchetActive: false,
      );
      if (mounted) {
        setState(() {
          _contacts = [
            for (final c in _contacts)
              if (c.name != (contact?.name ?? '') && c.name != newName) c,
            updatedContact,
          ]..sort((a, b) => a.name.compareTo(b.name));
          _sel = updatedContact;
          _selGroup = null;
          _msgs = const [];
          _error = null;
        });
      }
      try {
        await _refresh();
      } catch (refreshError) {
        _snack('Contact saved; refresh failed: $refreshError');
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      // showDialog completes before the route's reverse animation has fully
      // detached its TextFields. Android may rebuild them during teardown.
      await Future<void>.delayed(const Duration(milliseconds: 300));
      name.dispose();
      onion.dispose();
      pubkey.dispose();
      x25519.dispose();
    }
  }

  Future<void> _showCreateGroupDialog() async {
    final title = TextEditingController();
    final selected = <String>{};
    try {
      final ok = await showDialog<bool>(
        context: context,
        builder: (_) => StatefulBuilder(
          builder: (context, setDialogState) => AlertDialog(
            backgroundColor: _t.surface,
            title: Text('Create group', style: TextStyle(color: _t.text)),
            content: SizedBox(
              width: 520,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  TextField(
                    controller: title,
                    decoration: const InputDecoration(labelText: 'Group title'),
                  ),
                  const SizedBox(height: 12),
                  Text('Members',
                      style: TextStyle(color: _t.textDim, fontSize: 12)),
                  const SizedBox(height: 6),
                  if (_contacts.isEmpty)
                    Text('No contacts yet. Add contacts first.',
                        style: TextStyle(color: _t.textDim, fontSize: 12))
                  else
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxHeight: 260),
                      child: ListView(
                        shrinkWrap: true,
                        children: _contacts
                            .map((contact) => CheckboxListTile(
                                  value: selected.contains(contact.name),
                                  dense: true,
                                  title: Text(contact.name),
                                  subtitle: Text(contact.securityLabel),
                                  onChanged: (checked) {
                                    setDialogState(() {
                                      if (checked == true) {
                                        selected.add(contact.name);
                                      } else {
                                        selected.remove(contact.name);
                                      }
                                    });
                                  },
                                ))
                            .toList(),
                      ),
                    ),
                  const SizedBox(height: 8),
                  Text(
                    'Groups fan out one encrypted message per member. There is no shared group ratchet yet.',
                    style: TextStyle(color: _t.textDim, fontSize: 11),
                  ),
                ],
              ),
            ),
            actions: [
              TextButton(
                  onPressed: () => Navigator.pop(context, false),
                  child: const Text('Cancel')),
              FilledButton(
                  onPressed: () => Navigator.pop(context, true),
                  child: const Text('Create')),
            ],
          ),
        ),
      );
      if (ok != true) return;
      final groupTitle = title.text.trim();
      if (groupTitle.isEmpty) throw Exception('group title is required');
      if (selected.isEmpty) throw Exception('select at least one group member');
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.createGroup(
          title: groupTitle,
          members: selected.toList()..sort(),
        );
      } else {
        final group = await _cli.createGroup(
          title: groupTitle,
          members: selected.toList()..sort(),
        );
        _showInfo('Group created', group.details);
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      title.dispose();
    }
  }

  Future<void> _loadGroupHistory(String group) async {
    try {
      final h = _canUseMobileBackend && _mobile != null
          ? await _mobile!.history(group: group)
          : await _cli.history(group: group);
      setState(() => _msgs = _mergePending(h.msgs));
      _scrollToBottom();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _deleteContact(Contact contact) async {
    if (!await _confirm(
        'Delete contact', 'Delete ${contact.name}? Message history is kept.')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.deleteContact(contact.name);
      } else {
        _showInfo('Contact deleted', await _cli.deleteContact(contact.name));
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _acceptContact(Contact contact) async {
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.acceptContact(contact.name);
      } else {
        _showInfo('Contact accepted', await _cli.acceptContact(contact.name));
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _startRatchet(Contact contact) async {
    if (contact.ratchetActive) {
      _showInfo('Forward secrecy',
          'Double Ratchet is already active for ${contact.name}.');
      return;
    }
    if (!await _confirm('Enable forward secrecy',
        'Start a Double Ratchet session with ${contact.name}? Your next message will upgrade to forward-secret encryption. Both sides must be online for the ratchet to fully establish.')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.ratchet(contact.name);
      } else {
        await _cli.ratchet(contact.name);
      }
      await _refresh();
      _showInfo('Forward secrecy',
          'Double Ratchet initialized for ${contact.name}. Send a message to complete the handshake.');
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _blockContact(Contact contact) async {
    if (!await _confirm('Block contact',
        'Block ${contact.name}? Future inbound messages from this key/onion will be dropped.')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.blockContact(contact.name);
      } else {
        _showInfo('Contact blocked', await _cli.blockContact(contact.name));
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _unblockContact(Contact contact) async {
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.unblockContact(contact.name);
      } else {
        _showInfo('Contact unblocked', await _cli.unblockContact(contact.name));
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _clearHistoryFor(Contact contact) async {
    if (!await _confirm(
        'Delete history', 'Delete all message history for ${contact.name}?')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.clearHistory(contact: contact.name);
      } else {
        _showInfo(
            'History deleted', await _cli.clearHistory(contact: contact.name));
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _clearGroupHistoryFor(GroupInfo group) async {
    if (!await _confirm('Delete group history',
        'Delete all message history for ${group.title}?')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.clearHistory(contact: group.id);
      } else {
        _showInfo('History deleted', await _cli.clearHistory(group: group.id));
      }
      await _refresh();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _deleteGroup(GroupInfo group) async {
    if (!await _confirm('Delete group',
        'Delete ${group.title} and its local message history?')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        await _mobile!.deleteGroup(group.id);
      } else {
        _showInfo('Group deleted', await _cli.deleteGroup(group.id));
      }
      await _refresh();
      if (_selGroup?.id == group.id) {
        setState(() {
          _selGroup = null;
          _msgs = const [];
        });
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _showEditGroupDialog(GroupInfo group) async {
    final title = TextEditingController(text: group.title);
    final selected = group.members.toSet();
    try {
      final ok = await showDialog<bool>(
        context: context,
        builder: (_) => StatefulBuilder(
          builder: (context, setDialogState) => AlertDialog(
            backgroundColor: _t.surface,
            title:
                Text('Manage ${group.title}', style: TextStyle(color: _t.text)),
            content: SizedBox(
              width: 520,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  TextField(
                    controller: title,
                    decoration: const InputDecoration(labelText: 'Group title'),
                  ),
                  const SizedBox(height: 12),
                  Text('Members',
                      style: TextStyle(color: _t.textDim, fontSize: 12)),
                  const SizedBox(height: 6),
                  ConstrainedBox(
                    constraints: const BoxConstraints(maxHeight: 260),
                    child: ListView(
                      shrinkWrap: true,
                      children: _contacts
                          .map((contact) => CheckboxListTile(
                                value: selected.contains(contact.name),
                                dense: true,
                                title: Text(contact.name),
                                subtitle: Text(contact.securityLabel),
                                onChanged: (checked) {
                                  setDialogState(() {
                                    if (checked == true) {
                                      selected.add(contact.name);
                                    } else {
                                      selected.remove(contact.name);
                                    }
                                  });
                                },
                              ))
                          .toList(),
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Membership changes affect local fan-out for future sends. They do not delete old messages or enforce remote removals.',
                    style: TextStyle(color: _t.textDim, fontSize: 11),
                  ),
                ],
              ),
            ),
            actions: [
              TextButton(
                  onPressed: () => Navigator.pop(context, false),
                  child: const Text('Cancel')),
              FilledButton(
                  onPressed: () => Navigator.pop(context, true),
                  child: const Text('Save')),
            ],
          ),
        ),
      );
      if (ok != true) return;
      final newTitle = title.text.trim();
      if (newTitle.isEmpty) throw Exception('group title is required');
      if (selected.isEmpty) throw Exception('select at least one group member');
      final mobile = _canUseMobileBackend ? _mobile : null;
      if (newTitle != group.title) {
        if (mobile != null) {
          await mobile.renameGroup(groupId: group.id, title: newTitle);
        } else {
          await _cli.renameGroup(group: group.id, title: newTitle);
        }
      }
      final wanted = selected.toSet();
      final current = group.members.toSet();
      for (final member in wanted.difference(current)) {
        if (mobile != null) {
          await mobile.addGroupMember(groupId: group.id, member: member);
        } else {
          await _cli.addGroupMember(group: group.id, member: member);
        }
      }
      for (final member in current.difference(wanted)) {
        if (mobile != null) {
          await mobile.removeGroupMember(groupId: group.id, member: member);
        } else {
          await _cli.removeGroupMember(group: group.id, member: member);
        }
      }
      await _load();
      GroupInfo? updated;
      for (final candidate in _groups.where((g) => g.id == group.id)) {
        updated = candidate;
        break;
      }
      if (updated != null) {
        setState(() => _selGroup = updated);
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      title.dispose();
    }
  }

  Future<void> _showGroupMenu(GroupInfo group, Offset position) async {
    final action = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
          position.dx, position.dy, position.dx, position.dy),
      color: _t.surface,
      items: const [
        PopupMenuItem(value: 'history', child: Text('Show history')),
        PopupMenuItem(value: 'clear-history', child: Text('Delete history')),
        PopupMenuDivider(),
        PopupMenuItem(value: 'edit', child: Text('Manage group')),
        PopupMenuItem(value: 'delete', child: Text('Delete group')),
        PopupMenuItem(value: 'leave', child: Text('Leave group')),
        PopupMenuDivider(),
        PopupMenuItem(value: 'details', child: Text('Group details')),
      ],
    );
    if (action == null) return;
    await _handleGroupAction(group, action);
  }

  Future<void> _handleGroupAction(GroupInfo group, String action) async {
    switch (action) {
      case 'history':
        await _runSlashCommand('/history-group ${group.id}');
        return;
      case 'clear-history':
        await _clearGroupHistoryFor(group);
        return;
      case 'edit':
        await _showEditGroupDialog(group);
        return;
      case 'delete':
        await _deleteGroup(group);
        return;
      case 'leave':
        await _leaveGroup(group);
        return;
      case 'details':
        _showInfo('Group details', group.details);
        return;
    }
  }

  Future<void> _leaveGroup(GroupInfo group) async {
    if (!await _confirm('Leave group',
        'Leave ${group.title}? The group will remain for other members.')) {
      return;
    }
    try {
      if (_canUseMobileBackend && _mobile != null) {
        final left = await _mobile!.leaveGroup(group.id);
        _showInfo('Left group', left.details);
      } else {
        _showInfo('Left group', await _cli.leaveGroup(group.id));
      }
      await _load();
      if (_selGroup?.id == group.id) {
        setState(() {
          _selGroup = null;
          _msgs = const [];
        });
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _showContactMenu(Contact contact, Offset position) async {
    final action = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
          position.dx, position.dy, position.dx, position.dy),
      color: _t.surface,
      items: [
        const PopupMenuItem(value: 'history', child: Text('Show history')),
        const PopupMenuItem(
            value: 'clear-history', child: Text('Delete history')),
        const PopupMenuDivider(),
        PopupMenuItem(
          value: 'ratchet',
          enabled: !contact.ratchetActive,
          child: Row(
            children: [
              Icon(Icons.lock_outline,
                  size: 16,
                  color: contact.ratchetActive ? _t.primary : _t.textDim),
              const SizedBox(width: 8),
              Text(contact.ratchetActive
                  ? 'Forward secrecy active'
                  : 'Enable forward secrecy'),
            ],
          ),
        ),
        const PopupMenuDivider(),
        const PopupMenuItem(value: 'accept', child: Text('Add pending contact')),
        const PopupMenuItem(value: 'block', child: Text('Block contact')),
        const PopupMenuItem(value: 'unblock', child: Text('Unblock contact')),
        const PopupMenuDivider(),
        const PopupMenuItem(value: 'edit', child: Text('Edit contact')),
        const PopupMenuItem(value: 'delete', child: Text('Delete contact')),
        const PopupMenuDivider(),
        const PopupMenuItem(value: 'details', child: Text('Contact details')),
      ],
    );
    if (action == null) return;
    switch (action) {
      case 'history':
        await _runSlashCommand('/history ${contact.name}');
        return;
      case 'clear-history':
        await _clearHistoryFor(contact);
        return;
      case 'accept':
        await _acceptContact(contact);
        return;
      case 'ratchet':
        await _startRatchet(contact);
        return;
      case 'block':
        await _blockContact(contact);
        return;
      case 'unblock':
        await _unblockContact(contact);
        return;
      case 'edit':
        await _showEditContactDialog(contact);
        return;
      case 'delete':
        await _deleteContact(contact);
        return;
      case 'details':
        _showInfo('Contact details',
            '${contact.name}\nonion=${contact.onion}\npubkey=${contact.pubkey}\nx25519=${contact.x25519Pubkey}');
        return;
    }
  }

  Future<void> _changeDisplayName() async {
    final current = await _cli.name();
    if (!mounted) return;
    final controller = TextEditingController(text: current.trim());
    try {
      final ok = await showDialog<bool>(
        context: context,
        builder: (dialogContext) => AlertDialog(
          backgroundColor: _t.surface,
          title: Text('Display name', style: TextStyle(color: _t.text)),
          content: TextField(
            controller: controller,
            decoration: const InputDecoration(labelText: 'Name'),
          ),
          actions: [
            TextButton(
                onPressed: () => Navigator.pop(dialogContext, false),
                child: const Text('Cancel')),
            FilledButton(
                onPressed: () => Navigator.pop(dialogContext, true),
                child: const Text('Save')),
          ],
        ),
      );
      if (ok == true) {
        final result = await _cli.name(controller.text.trim());
        if (mounted) _showInfo('Name', result);
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      controller.dispose();
    }
  }

  String? _currentOnion() {
    final onion = _listenerOnion ?? _mobileOnion;
    if (onion == null || onion.trim().isEmpty) return null;
    final trimmed = onion.trim();
    if (trimmed.startsWith('(') || !trimmed.endsWith('.onion')) return null;
    return trimmed;
  }

  String _runtimeProfileLabel() {
    return _expandedProfilePath();
  }

  String _runtimeBackendLabel() {
    if (_canUseMobileBackend) return 'libsideband.so (Android FFI)';
    return _cli.bin;
  }

  Future<ShareInfo> _shareInfo() async {
    if (_canUseMobileBackend && _mobile != null) {
      await _syncMobileListenerStatus();
    }
    final onion = _currentOnion();
    if (onion == null) {
      throw Exception('onion address is not ready yet');
    }
    if (_canUseMobileBackend && _mobile != null) {
      return _mobile!.share(onion);
    }
    return _cli.share(onion);
  }

  Future<void> _showShareDialog() async {
    try {
      final share = await _shareInfo();
      if (!mounted) return;
      final screenWidth = MediaQuery.of(context).size.width;
      final dialogWidth = screenWidth < 600 ? screenWidth * 0.9 : 520.0;
      final qrSize = dialogWidth < 400 ? 200.0 : 280.0;
      await showDialog<void>(
        context: context,
        builder: (dialogContext) => AlertDialog(
          backgroundColor: _t.surface,
          title: Text('Share contact', style: TextStyle(color: _t.text)),
          content: SizedBox(
            width: dialogWidth,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Container(
                  padding: const EdgeInsets.all(16),
                  decoration: BoxDecoration(
                    color: Colors.white,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: SizedBox(
                    width: qrSize,
                    height: qrSize,
                    child: CustomPaint(painter: _QrPainter(share.qr)),
                  ),
                ),
                const SizedBox(height: 16),
                Text(
                  'Scan this QR code to add this contact, or copy the command below.',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: _t.textDim),
                ),
                const SizedBox(height: 12),
                SelectableText(
                  share.command,
                  style: TextStyle(
                    color: _t.text,
                    fontFamily: 'monospace',
                    fontSize: 12,
                  ),
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(dialogContext),
              child: const Text('Close'),
            ),
            FilledButton.icon(
              icon: const Icon(Icons.copy, size: 16),
              label: const Text('Copy'),
              onPressed: () async {
                await Clipboard.setData(ClipboardData(text: share.command));
                if (dialogContext.mounted) Navigator.pop(dialogContext);
                if (mounted) {
                  _showInfo('Share contact',
                      '${share.command}\n\nCopied to clipboard.');
                }
              },
            ),
          ],
        ),
      );
    } catch (e) {
      if (!mounted) return;
      final message = '$e';
      final notReady = message.contains('onion address is not ready yet');
      _showInfo(
        'Share contact',
        notReady
            ? 'Your Tor onion address is not ready yet. Wait for the listener status to change from onion pending to listening, then try Share again.'
            : message,
      );
    }
  }

  Future<void> _clearAllHistory() async {
    if (!await _confirm('Delete all history', 'Delete all message history?')) {
      return;
    }
    try {
      _showInfo('History deleted', await _cli.clearHistory());
      await _load();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _showSettings() async {
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
        builder: (dialogContext, setDialogState) => AlertDialog(
          backgroundColor: _t.surface,
          title: Text('Settings', style: TextStyle(color: _t.text)),
          content: SizedBox(
            // Fixed comfortable width on desktop; fill the dialog on mobile so
            // long subtitles wrap instead of overflowing a narrow screen.
            width: _canUseMobileBackend ? double.maxFinite : 560,
            child: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  SwitchListTile(
                    secondary: const Icon(Icons.notifications_active_outlined),
                    title: Text(_canUseMobileBackend
                        ? 'Message notifications'
                        : 'Desktop notifications'),
                    subtitle: Text(_canUseMobileBackend
                        ? 'Notify me of new messages while the app is in the background'
                        : 'System notification daemon popups'),
                    value: _showSystemNotifications,
                    onChanged: (value) {
                      setState(() => _showSystemNotifications = value);
                      setDialogState(() {});
                      if (value && _canUseMobileBackend) {
                        unawaited(_requestNotificationPermission());
                      }
                    },
                  ),
                  SwitchListTile(
                    secondary: const Icon(Icons.mark_unread_chat_alt_outlined),
                    title: const Text('In-app notification banner'),
                    subtitle: const Text('Show the top banner inside Sideband'),
                    value: _showInAppNotifications,
                    onChanged: (value) {
                      setState(() => _showInAppNotifications = value);
                      setDialogState(() {});
                    },
                  ),
                  // Audible notifications play through the desktop sound server;
                  // on Android the OS handles the channel's sound.
                  if (!_canUseMobileBackend)
                    SwitchListTile(
                      secondary: const Icon(Icons.volume_up_outlined),
                      title: const Text('Notification sound'),
                      subtitle: const Text('Play a sound on new messages'),
                      value: _showAudibleNotifications,
                      onChanged: (value) {
                        setState(() => _showAudibleNotifications = value);
                        setDialogState(() {});
                      },
                    ),
                  ListTile(
                    leading: const Icon(Icons.palette_outlined),
                    title: const Text('Theme'),
                    subtitle: Text(_selectedTheme),
                    onTap: () async {
                      Navigator.pop(dialogContext);
                      unawaited(_showThemePicker());
                    },
                  ),
                  // Window + tray controls are desktop-only.
                  if (!_canUseMobileBackend) ...[
                    const Divider(height: 20),
                    ListTile(
                      leading: const Icon(Icons.open_in_full),
                      title: const Text('Show Sideband'),
                      subtitle: const Text('Restore and focus the app window'),
                      onTap: () {
                        Navigator.pop(dialogContext);
                        unawaited(_showWindow());
                      },
                    ),
                    SwitchListTile(
                      secondary: const Icon(Icons.vertical_align_bottom),
                      title: const Text('Minimize to tray'),
                      subtitle:
                          const Text('Minimize button sends to system tray'),
                      value: _minimizeToTrayEnabled,
                      onChanged: (value) {
                        setState(() => _minimizeToTrayEnabled = value);
                        setDialogState(() {});
                      },
                    ),
                  ],
                  const Divider(height: 20),
                  ListTile(
                    leading: const Icon(Icons.badge_outlined),
                    title: const Text('Display name'),
                    subtitle: const Text('Set the name shared with contacts'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_changeDisplayName());
                    },
                  ),
                  ListTile(
                    leading: const Icon(Icons.person_add_alt_1),
                    title: const Text('Add contact'),
                    subtitle: const Text('Paste a shared /add command by hand'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_showAddContactDialog());
                    },
                  ),
                  ListTile(
                    leading: Icon(_canUseMobileBackend
                        ? Icons.qr_code_scanner
                        : Icons.upload_file),
                    title: Text(_canUseMobileBackend
                        ? 'Scan contact QR'
                        : 'Upload QR code image'),
                    subtitle: Text(_canUseMobileBackend
                        ? 'Use the camera to scan a shared Sideband QR'
                        : 'Choose a shared Sideband QR image from disk'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_canUseMobileBackend
                          ? _scanContactQr()
                          : _uploadContactQr());
                    },
                  ),
                  ListTile(
                    leading: const Icon(Icons.group_add),
                    title: const Text('Create group'),
                    subtitle: const Text(
                        'Pick contacts and make a local fan-out group'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_showCreateGroupDialog());
                    },
                  ),
                  ListTile(
                    leading: const Icon(Icons.ios_share),
                    title: const Text('Share my contact'),
                    subtitle: Text(_listenerStatus),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_showShareDialog());
                    },
                  ),
                  ListTile(
                    leading: const Icon(Icons.fingerprint),
                    title: const Text('Show identity'),
                    subtitle: const Text('Public keys and profile identity'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_runSlashCommand('/whoami'));
                    },
                  ),
                  ListTile(
                    leading: const Icon(Icons.info_outline),
                    title: const Text('Runtime status'),
                    subtitle: Text(
                        '${_runtimeProfileLabel()} • ${_contacts.length} contacts'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_runSlashCommand('/status'));
                    },
                  ),
                  if (_canUseMobileBackend)
                    ListTile(
                      leading: const Icon(Icons.swap_vert),
                      title: const Text('File transfers'),
                      subtitle:
                          const Text('Resume or cancel in-flight file transfers'),
                      onTap: () {
                        Navigator.pop(dialogContext);
                        unawaited(_showTransfersSheet());
                      },
                    ),
                  ListTile(
                    leading: const Icon(Icons.delete_sweep_outlined),
                    title: const Text('Delete all history'),
                    subtitle: const Text('Contacts stay. Messages go away.'),
                    onTap: () {
                      Navigator.pop(dialogContext);
                      unawaited(_clearAllHistory());
                    },
                  ),
                  if (!_listenerRunning)
                    ListTile(
                      leading: const Icon(Icons.power_settings_new),
                      title: const Text('Start listener'),
                      subtitle: const Text('Bring the onion service back up'),
                      onTap: () {
                        Navigator.pop(dialogContext);
                        unawaited(_canUseMobileBackend
                            ? _startMobileListener()
                            : _startListener());
                      },
                    ),
                ],
              ),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(dialogContext),
              child: const Text('Close'),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _showThemePicker() async {
    final chosen = await showDialog<String>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        backgroundColor: _t.surface,
        title: Text('Theme', style: TextStyle(color: _t.text)),
        content: SizedBox(
          width: 320,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: _themes.keys.map((name) {
              final td = _themes[name]!;
              return ListTile(
                leading: CircleAvatar(
                  radius: 12,
                  backgroundColor: td.primary,
                ),
                title: Text(name, style: TextStyle(color: _t.text)),
                selected: _selectedTheme == name,
                selectedTileColor: _t.selectedTile,
                onTap: () => Navigator.pop(dialogContext, name),
              );
            }).toList(),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: const Text('Cancel'),
          ),
        ],
      ),
    );
    if (chosen != null && chosen != _selectedTheme) {
      setState(() => _selectedTheme = chosen);
      widget.onThemeChanged(chosen);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      key: scaffoldKey,
      body: SafeArea(
        child: Column(
          children: [
            _notificationBanner(),
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  // GTK can hand Flutter a 1x1 surface before the first real frame.
                  // Rendering the full layout there just trips Flex overflow asserts.
                  if (constraints.maxWidth < 80 || constraints.maxHeight < 80) {
                    return ColoredBox(color: _t.bg);
                  }

                  if (_loading) {
                    return Center(
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          SizedBox(
                            width: 28,
                            height: 28,
                            child: CircularProgressIndicator(
                                strokeWidth: 2.5, color: _t.primary),
                          ),
                          const SizedBox(height: 16),
                          Text('Connecting…',
                              style: Theme.of(context)
                                  .textTheme
                                  .bodyMedium
                                  ?.copyWith(color: _t.textDim)),
                        ],
                      ),
                    );
                  }

                  if (constraints.maxWidth < 720) {
                    return _sel == null && _selGroup == null
                        ? _sidebar()
                        : _chat();
                  }

                  return Row(
                    children: [
                      SizedBox(width: 320, child: _sidebar()),
                      Container(width: 1, color: _t.border),
                      Expanded(
                          child: _sel == null && _selGroup == null
                              ? _emptyChat()
                              : _chat()),
                    ],
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }

  // ── sidebar ──────────────────────────────────────────────────────────────

  Widget _sidebar() {
    return Material(
      color: _t.surface,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // header
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 14, 14, 10),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    'Messages',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: _t.text,
                      fontSize: 17,
                      fontWeight: FontWeight.w700,
                      letterSpacing: 0.2,
                    ),
                  ),
                ),
                SizedBox(
                  width: 44,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints.tightFor(width: 44, height: 44),
                    icon: const Icon(Icons.person_add_alt_1, size: 20),
                    onPressed: _showAddContactDialog,
                    tooltip: 'Add contact',
                  ),
                ),
                SizedBox(
                  width: 44,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints.tightFor(width: 44, height: 44),
                    icon: const Icon(Icons.group_add, size: 20),
                    onPressed: _showCreateGroupDialog,
                    tooltip: 'Create group',
                  ),
                ),
                SizedBox(
                  width: 44,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints.tightFor(width: 44, height: 44),
                    icon: Icon(
                      _canUseMobileBackend
                          ? Icons.qr_code_scanner
                          : Icons.upload_file,
                      size: 20,
                    ),
                    onPressed: _canUseMobileBackend
                        ? _scanContactQr
                        : _uploadContactQr,
                    tooltip: _canUseMobileBackend
                        ? 'Scan contact QR'
                        : 'Upload QR code image',
                  ),
                ),
                SizedBox(
                  width: 44,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints.tightFor(width: 44, height: 44),
                    icon: const Icon(Icons.qr_code, size: 20),
                    onPressed: _showShareDialog,
                    tooltip: 'Share contact',
                  ),
                ),
                SizedBox(
                  width: 44,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints.tightFor(width: 44, height: 44),
                    icon: const Icon(Icons.settings, size: 20),
                    onPressed: _showSettings,
                    tooltip: 'Settings',
                  ),
                ),
                SizedBox(
                  width: 44,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints.tightFor(width: 44, height: 44),
                    icon: const Icon(Icons.refresh, size: 20),
                    onPressed: _refresh,
                    tooltip: 'Refresh',
                  ),
                ),
              ],
            ),
          ),
          // contacts
          Expanded(
            child: _error != null && _contacts.isEmpty
                ? _sidebarError()
                : _contacts.isEmpty && _groups.isEmpty
                    ? Center(
                        child: Padding(
                          padding: const EdgeInsets.all(24),
                          child: Text(
                            'No contacts yet.\nUse + or /add <name> <onion> <ed25519> <x25519>.\nCreate groups with the group-add button or /group-create.',
                            textAlign: TextAlign.center,
                            style: TextStyle(
                                color: _t.textDim, fontSize: 12, height: 1.6),
                          ),
                        ),
                      )
                    : ListView.builder(
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                        itemCount: _contacts.length +
                            (_groups.isEmpty ? 0 : _groups.length + 1),
                        itemBuilder: (_, i) {
                          if (i >= _contacts.length) {
                            if (i == _contacts.length) {
                              return Padding(
                                padding:
                                    const EdgeInsets.fromLTRB(14, 14, 14, 6),
                                child: Text('Groups',
                                    style: TextStyle(
                                        color: _t.primary,
                                        fontSize: 12,
                                        fontWeight: FontWeight.w700)),
                              );
                            }
                            final g = _groups[i - _contacts.length - 1];
                            return GestureDetector(
                              onSecondaryTapDown: (details) =>
                                  _showGroupMenu(g, details.globalPosition),
                              child: ListTile(
                                leading: CircleAvatar(
                                  radius: 17,
                                  backgroundColor: _t.surface2,
                                  child: Icon(Icons.groups,
                                      size: 17, color: _t.primary),
                                ),
                                title: Text(g.sidebarLabel,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: TextStyle(
                                        fontSize: 13.5,
                                        fontWeight: FontWeight.w500,
                                        color: _t.text)),
                                subtitle: Text(g.memberSummary,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: TextStyle(
                                        fontSize: 10.5, color: _t.textDim)),
                                trailing: Row(
                                  mainAxisSize: MainAxisSize.min,
                                  children: [
                                    if (_unreadGroups.contains(g.id))
                                      _unreadDot(),
                                    PopupMenuButton<String>(
                                      icon:
                                          const Icon(Icons.more_vert, size: 17),
                                      tooltip: 'Group menu',
                                      color: _t.surface,
                                      onSelected: (action) async =>
                                          _handleGroupAction(g, action),
                                      itemBuilder: (_) => const [
                                        PopupMenuItem(
                                            value: 'history',
                                            child: Text('Show history')),
                                        PopupMenuItem(
                                            value: 'clear-history',
                                            child: Text('Delete history')),
                                        PopupMenuDivider(),
                                        PopupMenuItem(
                                            value: 'edit',
                                            child: Text('Manage group')),
                                        PopupMenuItem(
                                            value: 'delete',
                                            child: Text('Delete group')),
                                        PopupMenuDivider(),
                                        PopupMenuItem(
                                            value: 'details',
                                            child: Text('Group details')),
                                      ],
                                    ),
                                  ],
                                ),
                                selected: _selGroup?.id == g.id,
                                onTap: () async {
                                  try {
                                    final h =
                                        await _historyDispatch(null, group: g.id);
                                    if (!mounted) return;
                                    setState(() {
                                      _sel = null;
                                      _selGroup = g;
                                      _msgs = _mergePending(h.msgs);
                                      _unreadGroups.remove(g.id);
                                    });
                                    _scrollToBottom();
                                  } catch (e) {
                                    if (mounted) setState(() => _error = '$e');
                                  }
                                },
                              ),
                            );
                          }
                          final c = _contacts[i];
                          final on = _sel?.name == c.name;
                          final lastMsg = _lastMsg[c.name];
                          final presenceText = _presenceLabel(c.name);
                          return GestureDetector(
                            onSecondaryTapDown: (details) =>
                                _showContactMenu(c, details.globalPosition),
                            child: ListTile(
                              selected: on,
                              leading: Stack(
                                clipBehavior: Clip.none,
                                children: [
                                  CircleAvatar(
                                    radius: 17,
                                    backgroundColor: c.avatarColor,
                                    child: Text(c.initial,
                                        style: const TextStyle(
                                            color: Colors.white,
                                            fontSize: 12,
                                            fontWeight: FontWeight.w700)),
                                  ),
                                  Positioned(
                                    bottom: -1,
                                    right: -1,
                                    child: _presenceDot(c.name),
                                  ),
                                ],
                              ),
                              title: Row(
                                children: [
                                  Expanded(
                                    child: Text(c.name,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: TextStyle(
                                          fontSize: 13.5,
                                          fontWeight: on
                                              ? FontWeight.w600
                                              : FontWeight.w500,
                                          color: on ? _t.primary : _t.text,
                                        )),
                                  ),
                                  if (lastMsg != null)
                                    Text(
                                      _hm(lastMsg.ts),
                                          style: TextStyle(
                                              fontSize: 10, color: _t.textDim),
                                    ),
                                  const SizedBox(width: 6),
                                  Tooltip(
                                    message: c.securityDescription,
                                    child: Icon(_securityIcon(c),
                                        size: 13, color: _securityColor(c)),
                                  ),
                                ],
                              ),
                              subtitle: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  if (lastMsg != null)
                                    Text(
                                      _previewText(lastMsg),
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: TextStyle(
                                          fontSize: 11, color: _t.textDim),
                                    )
                                  else
                                    Text(
                                      c.shortOnion,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: TextStyle(
                                          fontSize: 10.5, color: _t.textDim),
                                    ),
                                  const SizedBox(height: 1),
                                  Text(presenceText,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: TextStyle(
                                          fontSize: 10,
                                          color: _online[c.name] == true
                                              ? _t.primary
                                              : _t.textDim)),
                                ],
                              ),
                              trailing: Row(
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  if (_unreadContacts.contains(c.name))
                                    _unreadDot(),
                                  PopupMenuButton<String>(
                                    tooltip: 'Contact menu',
                                    icon: const Icon(Icons.more_vert, size: 17),
                                    color: _t.surface,
                                    onSelected: (action) async {
                                      switch (action) {
                                        case 'history':
                                          await _runSlashCommand(
                                              '/history ${c.name}');
                                          return;
                                        case 'clear-history':
                                          await _clearHistoryFor(c);
                                          return;
                                        case 'ratchet':
                                          await _startRatchet(c);
                                          return;
                                        case 'accept':
                                          await _acceptContact(c);
                                          return;
                                        case 'block':
                                          await _blockContact(c);
                                          return;
                                        case 'unblock':
                                          await _unblockContact(c);
                                          return;
                                        case 'edit':
                                          await _showEditContactDialog(c);
                                          return;
                                        case 'delete':
                                          await _deleteContact(c);
                                          return;
                                        case 'details':
                                          _showInfo('Contact details',
                                              '${c.name}\nsecurity=${c.securityLabel}\nonion=${c.onion}\npubkey=${c.pubkey}\nx25519=${c.x25519Pubkey}');
                                          return;
                                      }
                                    },
                                    itemBuilder: (_) => [
                                      const PopupMenuItem(
                                          value: 'history',
                                          child: Text('Show history')),
                                      const PopupMenuItem(
                                          value: 'clear-history',
                                          child: Text('Delete history')),
                                      const PopupMenuDivider(),
                                      PopupMenuItem(
                                        value: 'ratchet',
                                        enabled: !c.ratchetActive,
                                        child: Row(
                                          children: [
                                            Icon(Icons.lock_outline,
                                                size: 16,
                                                color: c.ratchetActive
                                                    ? _t.primary
                                                    : _t.textDim),
                                            const SizedBox(width: 8),
                                            Text(c.ratchetActive
                                                ? 'Forward secrecy active'
                                                : 'Enable forward secrecy'),
                                          ],
                                        ),
                                      ),
                                      const PopupMenuDivider(),
                                      const PopupMenuItem(
                                          value: 'accept',
                                          child: Text('Add pending contact')),
                                      const PopupMenuItem(
                                          value: 'block',
                                          child: Text('Block contact')),
                                      const PopupMenuItem(
                                          value: 'unblock',
                                          child: Text('Unblock contact')),
                                      const PopupMenuDivider(),
                                      const PopupMenuItem(
                                          value: 'edit',
                                          child: Text('Edit contact')),
                                      const PopupMenuItem(
                                          value: 'delete',
                                          child: Text('Delete contact')),
                                      const PopupMenuDivider(),
                                      const PopupMenuItem(
                                          value: 'details',
                                          child: Text('Contact details')),
                                    ],
                                  ),
                                ],
                              ),
                              onTap: () async {
                                setState(() {
                                  _sel = c;
                                  _selGroup = null;
                                  _unreadContacts.remove(c.name);
                                });
                                await _refresh();
                              },
                            ),
                          );
                        },
                      ),
          ),
          // footer
          Container(
            padding: const EdgeInsets.all(10),
            decoration: BoxDecoration(
              border: Border(top: BorderSide(color: _t.border, width: 1)),
            ),
            child: Row(
              children: [
                Container(
                  width: 7,
                  height: 7,
                  decoration: BoxDecoration(
                      color: _listenerRunning ? _t.primary : _t.errorFg,
                      shape: BoxShape.circle),
                ),
                const SizedBox(width: 6),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        _listenerStatus,
                        style: TextStyle(
                            fontSize: 10,
                            color: _listenerRunning ? _t.primary : _t.errorFg),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                      Text(
                        _runtimeProfileLabel(),
                        style: TextStyle(fontSize: 10, color: _t.textDim),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ],
                  ),
                ),
                if (!_listenerRunning)
                  IconButton(
                    icon: const Icon(Icons.power_settings_new, size: 16),
                    tooltip: 'Start listener',
                    onPressed: _canUseMobileBackend
                        ? _startMobileListener
                        : _startListener,
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  // ── empty state ──────────────────────────────────────────────────────────

  Widget _sidebarError() {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: _t.errorBg,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: _t.errorFg.withAlpha(90)),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.warning_amber_rounded, size: 16, color: _t.errorFg),
                const SizedBox(width: 6),
                Text('Backend error',
                    style: TextStyle(
                        color: _t.errorFg,
                        fontSize: 12,
                        fontWeight: FontWeight.w700)),
              ],
            ),
            const SizedBox(height: 8),
            Text(_error!, style: TextStyle(color: _t.errorFg, fontSize: 11.5)),
            const SizedBox(height: 12),
            OutlinedButton.icon(
              onPressed: _refresh,
              icon: const Icon(Icons.refresh, size: 14),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }

  Widget _emptyChat() {
    return Container(
      color: _t.bg,
      child: Column(
        children: [
          Expanded(
            child: Center(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(Icons.send_rounded,
                      size: 42, color: _t.textDim.withAlpha(50)),
                  const SizedBox(height: 14),
                  Text('No conversation selected',
                      style: TextStyle(color: _t.textDim, fontSize: 14)),
                  const SizedBox(height: 6),
                  Text('Use slash commands below, for example /add or /help.',
                      textAlign: TextAlign.center,
                      style: TextStyle(color: _t.textDim, fontSize: 12)),
                ],
              ),
            ),
          ),
          Container(height: 1, color: _t.border),
          _inputArea(),
        ],
      ),
    );
  }

  // ── chat ─────────────────────────────────────────────────────────────────

  Widget _chat() {
    return Container(
      color: _t.bg,
      child: Column(
        children: [
          _chatHeader(),
          Container(height: 1, color: _t.border),
          if (_error != null) _errorBanner(),
          if (_sel?.pending == true) _pendingContactBanner(_sel!),
          if (_sel?.blocked == true) _blockedContactBanner(_sel!),
          if (_retryQueued > 0) _retryQueueBanner(),
          Expanded(child: _msgList()),
          Container(height: 1, color: _t.border),
          _inputArea(),
        ],
      ),
    );
  }

  Widget _pendingContactBanner(Contact contact) {
    return Container(
      width: double.infinity,
      color: const Color(0xFF3D320F),
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
      child: Row(
        children: [
          const Icon(Icons.person_add_disabled_outlined,
              size: 16, color: Color(0xFFFFC857)),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              'Unknown sender. Add this contact to trust future messages, or block it.',
              style: TextStyle(color: _t.text, fontSize: 11.5),
            ),
          ),
          TextButton(
              onPressed: () => _acceptContact(contact),
              child: const Text('Add')),
          TextButton(
              onPressed: () => _blockContact(contact),
              child: const Text('Block')),
        ],
      ),
    );
  }

  Widget _blockedContactBanner(Contact contact) {
    return Container(
      width: double.infinity,
      color: _t.errorBg,
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
      child: Row(
        children: [
          Icon(Icons.block, size: 16, color: _t.errorFg),
          const SizedBox(width: 8),
          Expanded(
            child: Text('Blocked contact. Inbound messages are dropped.',
                style: TextStyle(color: _t.errorFg, fontSize: 11.5)),
          ),
          TextButton(
              onPressed: () => _unblockContact(contact),
              child: const Text('Unblock')),
        ],
      ),
    );
  }

  Widget _retryQueueBanner() {
    final n = _retryQueued;
    return Container(
      width: double.infinity,
      color: const Color(0xFF2D2A0F),
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
      child: Row(
        children: [
          const Icon(Icons.schedule_send_outlined, size: 16, color: Color(0xFFFFC857)),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              n == 1
                  ? '1 message queued for retry — recipient may be offline'
                  : '$n messages queued for retry — recipients may be offline',
              style: TextStyle(color: _t.text, fontSize: 11.5),
            ),
          ),
          TextButton(
            onPressed: _queryRetryStatus,
            child: const Text('Refresh'),
          ),
        ],
      ),
    );
  }

  Widget _chatHeader() {
    final c = _sel;
    final g = _selGroup;
    final title = c?.name ?? g!.sidebarLabel;
    final isNarrow = MediaQuery.of(context).size.width < 720;
    final presenceText = c != null ? _presenceLabel(c.name) : '';
    final subtitle = c != null
        ? (presenceText.isNotEmpty ? presenceText : c.securityLabel)
        : 'Group fan-out to ${g!.memberSummary}';
    return Container(
      color: _t.surface,
      padding:
          EdgeInsets.symmetric(horizontal: isNarrow ? 8 : 18, vertical: 10),
      child: Row(
        children: [
          if (isNarrow)
            IconButton(
              icon: const Icon(Icons.arrow_back, size: 20),
              onPressed: () {
                setState(() {
                  _sel = null;
                  _selGroup = null;
                  _msgs = const [];
                });
              },
            ),
          Stack(
            clipBehavior: Clip.none,
            children: [
              CircleAvatar(
                radius: 16,
                backgroundColor: c?.avatarColor ?? _t.primary.withAlpha(110),
                child: Text(c?.initial ?? 'G',
                    style: const TextStyle(
                        color: Colors.white,
                        fontSize: 11,
                        fontWeight: FontWeight.w700)),
              ),
              if (c != null)
                Positioned(
                  bottom: -1,
                  right: -1,
                  child: _presenceDot(c.name),
                ),
            ],
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title,
                    style: TextStyle(
                        color: _t.text,
                        fontSize: 14.5,
                        fontWeight: FontWeight.w700)),
                const SizedBox(height: 1),
                Tooltip(
                  message: c?.securityDescription ??
                      'Group messages are sent separately to each member using their contact crypto.',
                  child: Row(
                    children: [
                      Icon(c == null ? Icons.groups_rounded : _securityIcon(c),
                          size: 11,
                          color: c == null ? _t.primary : _securityColor(c)),
                      const SizedBox(width: 4),
                      Text(subtitle,
                          style: TextStyle(
                              fontSize: 10.5,
                              color: c == null
                                  ? _t.primary
                                  : (_online[c.name] == true
                                      ? _t.primary
                                      : _securityColor(c)))),
                    ],
                  ),
                ),
              ],
            ),
          ),
          IconButton(
            icon: const Icon(Icons.history, size: 18),
            tooltip: 'History',
            onPressed: () => c == null
                ? _runSlashCommand('/history-group ${g!.id}')
                : _runSlashCommand('/history ${c.name}'),
          ),
          IconButton(
            icon: const Icon(Icons.delete_sweep_outlined, size: 18),
            tooltip: 'Delete history',
            onPressed: c == null
                ? (_selGroup == null
                    ? null
                    : () => _clearGroupHistoryFor(_selGroup!))
                : () => _clearHistoryFor(c),
          ),
          Tooltip(
            message: _listenerStatus,
            child: Container(
              width: 7,
              height: 7,
              decoration: BoxDecoration(
                color: _listenerRunning ? _t.primary : _t.errorFg,
                shape: BoxShape.circle,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _errorBanner() {
    return GestureDetector(
      onTap: () => setState(() => _error = null),
      child: Container(
        width: double.infinity,
        color: _t.errorBg,
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
        child: Row(
          children: [
            Icon(Icons.error_outline, size: 14, color: _t.errorFg),
            const SizedBox(width: 6),
            Expanded(
              child: Text(_error!,
                  style: TextStyle(color: _t.errorFg, fontSize: 11.5)),
            ),
            Icon(Icons.close, size: 13, color: _t.errorFg),
          ],
        ),
      ),
    );
  }

  // ── messages ─────────────────────────────────────────────────────────────

  Widget _msgList() {
    if (_msgs.isEmpty) {
      return Center(
          child: Text('No messages yet',
              style: TextStyle(color: _t.textDim, fontSize: 13)));
    }
    return ListView.builder(
      controller: _scroll,
      reverse: false,
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 14),
      itemCount: _msgs.length,
      itemBuilder: (ctx, i) {
        final m = _msgs[i];
        final prev = i > 0 ? _msgs[i - 1] : null;
        final showDate = prev == null || !_sameDay(m.ts, prev.ts);
        Widget? timeGap;
        if (prev != null &&
            m.tsMs - prev.tsMs > 15 * 60 * 1000 &&
            _sameDay(m.ts, prev.ts)) {
          timeGap = _timeGapLabel(m.ts, prev.ts);
        }
        return Column(
          children: [
            if (showDate) _dateLabel(m.ts),
            if (timeGap != null) timeGap,
            _bubble(m),
            const SizedBox(height: 3),
          ],
        );
      },
    );
  }

  Widget _dateLabel(DateTime dt) {
    final label = _dateString(dt);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 10),
      child: Row(
        children: [
          Expanded(child: Container(height: 1, color: _t.border)),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Text(label,
                style: TextStyle(
                    fontSize: 10.5,
                    color: _t.textDim,
                    fontWeight: FontWeight.w600,
                    letterSpacing: 0.3)),
          ),
          Expanded(child: Container(height: 1, color: _t.border)),
        ],
      ),
    );
  }

  Widget _timeGapLabel(DateTime current, DateTime previous) {
    final diff = current.difference(previous);
    String label;
    if (diff.inHours >= 1) {
      label = '${diff.inHours} hour${diff.inHours > 1 ? 's' : ''} later';
    } else {
      label = '${diff.inMinutes} min later';
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Center(
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
          decoration: BoxDecoration(
            color: _t.surface,
            borderRadius: BorderRadius.circular(10),
          ),
          child: Text(label,
              style: TextStyle(
                  fontSize: 10, color: _t.textDim, letterSpacing: 0.2)),
        ),
      ),
    );
  }

  String _displayText(ChatMsg m) {
    final payload = parseGroupPayloadText(m.text);
    if (payload != null) return payload.body;
    return m.text;
  }

  Widget _bubble(ChatMsg m) {
    final right = m.out;
    final showGroupSender = _selGroup != null;
    final displayText = _displayText(m);
    final attachment = parseAttachmentText(displayText);
    final timeLabel = Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(_hm(m.ts),
              style: TextStyle(
                  fontSize: 11,
                  color: _t.textDim,
                  fontWeight: FontWeight.w500)),
          if (right) ...[
            const SizedBox(width: 4),
            _statusIcon(m),
          ],
        ],
      ),
    );

    final content = Container(
      constraints:
          BoxConstraints(maxWidth: MediaQuery.of(context).size.width * 0.62),
      padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 9),
      decoration: BoxDecoration(
        color: right ? _t.bubbleOut : _t.bubbleIn,
        borderRadius: BorderRadius.only(
          topLeft: const Radius.circular(14),
          topRight: const Radius.circular(14),
          bottomLeft: Radius.circular(right ? 14 : 3),
          bottomRight: Radius.circular(right ? 3 : 14),
        ),
        border: attachment != null
            ? Border.all(color: _t.primary.withAlpha(70))
            : null,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (showGroupSender) ...[
            Text(right ? 'You' : m.contact,
                style: TextStyle(
                    color: right ? _t.primary : _t.textDim,
                    fontSize: 11,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 3),
          ],
          if (attachment == null)
            SelectableText(displayText,
                style: TextStyle(color: _t.text, fontSize: 14, height: 1.35))
          else
            _attachmentBubble(attachment),
        ],
      ),
    );

    // Timestamp sits to the left of the bubble in both directions, so it stays
    // out of the way of the message text and is easy to scan down the margin.
    final bubble = Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
      child: Row(
        mainAxisAlignment:
            right ? MainAxisAlignment.end : MainAxisAlignment.start,
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          timeLabel,
          const SizedBox(width: 8),
          Flexible(child: content),
        ],
      ),
    );

    if (m.failed && m.out) {
      return GestureDetector(
        onTap: () => _retryFailedMessage(m),
        child: Tooltip(
          message: 'Tap to retry',
          child: bubble,
        ),
      );
    }
    return bubble;
  }

  Widget _attachmentBubble(AttachmentInfo attachment) {
    final exists =
        attachment.path.isNotEmpty && File(attachment.path).existsSync();
    final preview = attachment.image && exists;
    return InkWell(
      onTap: exists ? () => _showAttachment(attachment) : null,
      borderRadius: BorderRadius.circular(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (preview) ...[
            ClipRRect(
              borderRadius: BorderRadius.circular(10),
              child: Image.file(
                File(attachment.path),
                width: 260,
                height: 180,
                fit: BoxFit.cover,
                errorBuilder: (_, __, ___) => _fileTile(attachment, exists),
              ),
            ),
            const SizedBox(height: 8),
          ],
          _fileTile(attachment, exists),
        ],
      ),
    );
  }

  Widget _fileTile(AttachmentInfo attachment, bool canOpen) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(
          attachment.image
              ? Icons.image_outlined
              : Icons.insert_drive_file_outlined,
          size: 18,
          color: _t.primary,
        ),
        const SizedBox(width: 8),
        Flexible(
          child: Text(
            attachment.label.isEmpty ? 'file' : attachment.label,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: canOpen ? _t.text : _t.textDim,
              fontSize: 13,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
      ],
    );
  }

  void _showAttachment(AttachmentInfo attachment) {
    if (attachment.image &&
        attachment.path.isNotEmpty &&
        File(attachment.path).existsSync()) {
      // Image file exists locally — show inline preview with zoom
      showDialog<void>(
        context: context,
        builder: (ctx) => Dialog(
          backgroundColor: _t.bg,
          insetPadding: const EdgeInsets.all(18),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 900, maxHeight: 720),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(14, 10, 8, 8),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          attachment.label,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                              color: _t.text, fontWeight: FontWeight.w700),
                        ),
                      ),
                      IconButton(
                        icon: const Icon(Icons.close),
                        onPressed: () => Navigator.pop(ctx),
                      ),
                    ],
                  ),
                ),
                Flexible(
                  child: InteractiveViewer(
                    minScale: 0.5,
                    maxScale: 5,
                    child:
                        Image.file(File(attachment.path), fit: BoxFit.contain),
                  ),
                ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(14, 8, 14, 12),
                  child: SelectableText(
                    attachment.path,
                    style: TextStyle(color: _t.textDim, fontSize: 11),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
      return;
    }

    // File exists locally but isn't previewed inline — hand it to the platform.
    if (attachment.path.isNotEmpty && File(attachment.path).existsSync()) {
      if (Platform.isAndroid) {
        // Only received files (which live under .sideband/downloads/) can be
        // opened; the Kotlin side rejects anything else. Sent-file rows point at
        // arbitrary picker paths — for those just surface the path instead of a
        // guaranteed rejection.
        final profile = _mobileProfilePath ?? '';
        if (!isUnderDownloadsDir(attachment.path, profile)) {
          _snack(attachment.path);
          return;
        }
        unawaited(_nativeChannel.invokeMethod<void>('openFile', {
          'path': attachment.path,
        }).catchError((Object e) {
          if (e is PlatformException && e.code == 'open_file_rejected') {
            _snack('File is outside the allowed folder');
          } else {
            _snack('Could not open: ${attachment.label}');
          }
        }));
      } else {
        unawaited(Process.run('xdg-open', [attachment.path]).then((r) {
          if (r.exitCode != 0) {
            _snack('Could not open: ${attachment.label}');
          }
        }).catchError((_) {
          _snack('Could not open: ${attachment.label}');
        }));
      }
      return;
    }

    // File path is empty or doesn't exist — show path info
    _snack(attachment.path.isEmpty ? attachment.label : attachment.path);
  }

  Widget _statusIcon(ChatMsg m) {
    if (m.sending) {
      return SizedBox(
        width: 9,
        height: 9,
        child: CircularProgressIndicator(
            strokeWidth: 1.5, color: _t.textDim.withAlpha(120)),
      );
    }
    if (m.failed) {
      return Icon(Icons.error_outline, size: 12, color: _t.errorFg);
    }
    // Determine if our message was likely read: any later inbound from same contact
    final wasRead = _wasRead(m);
    if (m.status == 'delivered' || wasRead) {
      return Icon(Icons.done_all, size: 13,
          color: wasRead ? _t.primary : _t.primary.withAlpha(140));
    }
    return Icon(Icons.done, size: 13, color: _t.primary.withAlpha(160));
  }

  bool _wasRead(ChatMsg sentMsg) {
    if (!sentMsg.out) return false;
    final contact = sentMsg.contact;
    if (contact.isEmpty) return false;
    // Check if we have any inbound message from this contact after this one
    for (final m in _msgs) {
      if (m.direction == 'in' && m.contact == contact && m.tsMs > sentMsg.tsMs) {
        return true;
      }
    }
    return false;
  }

  // ── input ────────────────────────────────────────────────────────────────

  static const Map<String, List<String>> _emojiSections = {
    'Smileys': [
      '😀',
      '😄',
      '😂',
      '🙂\uFE0F',
      '😊\uFE0F',
      '😍',
      '😘',
      '😎',
      '🤔',
      '😬\uFE0F',
      '😢',
      '😡'
    ],
    'Gestures': [
      '👍',
      '👎',
      '👋',
      '🙏',
      '🙌',
      '👏',
      '🤝',
      '💪',
      '🤘',
      '🫡',
      '👌',
      '✌\uFE0F'
    ],
    'Chat': [
      '❤\uFE0F',
      '🔥',
      '🎉',
      '✨',
      '⭐',
      '✅',
      '❌',
      '⚠\uFE0F',
      '💡',
      '📌',
      '💬',
      '🚀'
    ],
    'Objects': [
      '🔒',
      '🔑',
      '🛡\uFE0F',
      '🐛',
      '🎯',
      '🏆',
      '📷',
      '🖼\uFE0F',
      '📎',
      '📁',
      '☕',
      '🌙'
    ],
  };

  void _insertEmoji(String emoji) {
    final sel = _input.selection;
    final text = _input.text;
    final start = sel.isValid ? sel.start : text.length;
    final end = sel.isValid ? sel.end : text.length;
    final newText = text.replaceRange(start, end, emoji);
    _input.value = TextEditingValue(
      text: newText,
      selection: TextSelection.collapsed(offset: start + emoji.length),
    );
  }

  void _showEmojiPicker() {
    showModalBottomSheet<void>(
      context: context,
      backgroundColor: _t.surface,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(16)),
      ),
      builder: (ctx) {
        return SafeArea(
          child: Padding(
            padding: EdgeInsets.only(
              bottom: MediaQuery.of(ctx).viewInsets.bottom,
            ),
            child: SizedBox(
              width: 400,
              height: 340,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(12, 10, 12, 12),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Icon(Icons.emoji_emotions_outlined,
                            size: 16, color: _t.primary),
                        const SizedBox(width: 6),
                        Text('Emoji',
                            style: TextStyle(
                                color: _t.text,
                                fontWeight: FontWeight.w700,
                                fontSize: 12)),
                        const Spacer(),
                        IconButton(
                          icon: const Icon(Icons.close, size: 16),
                          onPressed: () => Navigator.pop(ctx),
                          constraints:
                              const BoxConstraints(minWidth: 28, minHeight: 28),
                          padding: EdgeInsets.zero,
                        ),
                      ],
                    ),
                    const SizedBox(height: 4),
                    Expanded(
                      child: ListView(
                        children: _emojiSections.entries.map((entry) {
                          return Padding(
                            padding: const EdgeInsets.only(top: 8),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Padding(
                                  padding:
                                      const EdgeInsets.only(left: 4, bottom: 4),
                                  child: Text(entry.key,
                                      style: TextStyle(
                                          color: _t.textDim,
                                          fontSize: 10,
                                          fontWeight: FontWeight.w700,
                                          letterSpacing: 0.3)),
                                ),
                                Wrap(
                                  spacing: 4,
                                  runSpacing: 4,
                                  children: entry.value.map((emoji) {
                                    return Material(
                                      color: _t.surface2,
                                      borderRadius: BorderRadius.circular(6),
                                      child: InkWell(
                                        borderRadius: BorderRadius.circular(6),
                                        onTap: () {
                                          _insertEmoji(emoji);
                                          Navigator.pop(ctx);
                                        },
                                        child: SizedBox(
                                          width: 36,
                                          height: 34,
                                          child: Center(
                                            child: Text(emoji,
                                                style: const TextStyle(
                                                    fontSize: 20,
                                                    fontFamily:
                                                        'SidebandEmoji')),
                                          ),
                                        ),
                                      ),
                                    );
                                  }).toList(growable: false),
                                ),
                              ],
                            ),
                          );
                        }).toList(growable: false),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _inputArea() {
    final hasAttach = _pendingAttachmentPath != null;
    final contactBlocked = _sel?.blocked == true;
    final contactPending = _sel?.pending == true;
    final canSend = !_sending && !contactBlocked && !contactPending;
    return Container(
      color: _t.surface,
      padding: const EdgeInsets.fromLTRB(14, 10, 14, 12),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (hasAttach)
            Padding(
              padding: const EdgeInsets.only(bottom: 6),
              child: Row(
                children: [
                  Icon(Icons.attach_file, size: 14, color: _t.primary),
                  const SizedBox(width: 6),
                  Expanded(
                    child: Text(
                      _pendingAttachmentName ?? '',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                          color: _t.text,
                          fontSize: 12,
                          fontWeight: FontWeight.w600),
                    ),
                  ),
                  if (_pendingAttachmentSize > 0)
                    Text(
                      _formatBytes(_pendingAttachmentSize),
                      style: TextStyle(color: _t.textDim, fontSize: 10),
                    ),
                  const SizedBox(width: 4),
                  IconButton(
                    icon: const Icon(Icons.close, size: 14),
                    onPressed: _clearPendingAttachment,
                    padding: EdgeInsets.zero,
                    constraints:
                        const BoxConstraints(minWidth: 24, minHeight: 24),
                    splashRadius: 12,
                  ),
                ],
              ),
            ),
          Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              // emoji button
              Padding(
                padding: const EdgeInsets.only(bottom: 2),
                child: IconButton(
                  icon: const Text('😊', style: TextStyle(fontSize: 20)),
                  onPressed: _showEmojiPicker,
                  padding: EdgeInsets.zero,
                  constraints:
                      const BoxConstraints(minWidth: 32, minHeight: 32),
                  splashRadius: 18,
                ),
              ),
              const SizedBox(width: 4),
              // attachment button
              Padding(
                padding: const EdgeInsets.only(bottom: 2),
                child: IconButton(
                  icon: Icon(
                    hasAttach ? Icons.attach_file : Icons.attach_file_outlined,
                    size: 18,
                    color: hasAttach ? _t.primary : _t.textDim,
                  ),
                  onPressed: _showFileDialog,
                  padding: EdgeInsets.zero,
                  constraints:
                      const BoxConstraints(minWidth: 32, minHeight: 32),
                  splashRadius: 18,
                ),
              ),
              const SizedBox(width: 4),
              Expanded(
                child: Shortcuts(
                  shortcuts: const <ShortcutActivator, Intent>{
                    SingleActivator(LogicalKeyboardKey.enter):
                        _SendMessageIntent(),
                  },
                  child: Actions(
                    actions: <Type, Action<Intent>>{
                      _SendMessageIntent: CallbackAction<_SendMessageIntent>(
                        onInvoke: (_) {
                          if (canSend) unawaited(_send());
                          return null;
                        },
                      ),
                    },
                    child: TextField(
                      controller: _input,
                      enabled: canSend,
                      minLines: 1,
                      maxLines: 4,
                      keyboardType: TextInputType.multiline,
                      style: TextStyle(fontSize: 14, color: _t.text),
                      decoration: InputDecoration(
                        hintText: hasAttach
                            ? 'Add a message or send as-is…'
                            : contactBlocked
                                ? 'Contact is blocked'
                                : contactPending
                                    ? 'Add or block this contact first'
                                    : 'Message or /help for commands…',
                        contentPadding: const EdgeInsets.symmetric(
                            horizontal: 12, vertical: 8),
                      ),
                      textInputAction: TextInputAction.newline,
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              FilledButton(
                onPressed: canSend ? _send : null,
                style: FilledButton.styleFrom(
                  minimumSize: const Size(42, 42),
                  padding: EdgeInsets.zero,
                ),
                child: _sending
                    ? SizedBox(
                        width: 16,
                        height: 16,
                        child: CircularProgressIndicator(
                            strokeWidth: 2, color: _t.bg.withAlpha(140)),
                      )
                    : const Icon(Icons.send_rounded, size: 17),
              ),
            ],
          ),
        ],
      ),
    );
  }

  void _seedSeenIds(_History h) {
    _refreshSeenIds.addAll(h.msgs.map((m) => m.id));
  }

  // ── unread indicator ──────────────────────────────────────────────────────

  Widget _unreadDot() {
    return Container(
      width: 10,
      height: 10,
      margin: const EdgeInsets.only(right: 6),
      decoration: const BoxDecoration(
        color: Colors.redAccent,
        shape: BoxShape.circle,
      ),
    );
  }

  // ── utils ─────────────────────────────────────────────────────────────────

  String _hm(DateTime d) =>
      '${d.hour.toString().padLeft(2, '0')}:${d.minute.toString().padLeft(2, '0')}';

  bool _sameDay(DateTime a, DateTime b) =>
      a.year == b.year && a.month == b.month && a.day == b.day;

  String _dateString(DateTime d) {
    final n = DateTime.now();
    if (_sameDay(d, n)) return 'Today';
    final y = n.subtract(const Duration(days: 1));
    if (_sameDay(d, y)) return 'Yesterday';
    return '${d.month}/${d.day}/${d.year % 100}';
  }
}
