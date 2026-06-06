import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

// ── palette ────────────────────────────────────────────────────────────────

const _teal = Color(0xFF26D9C8);
const _bg = Color(0xFF0E1117);
const _surface = Color(0xFF161B22);
const _surface2 = Color(0xFF1C2333);
const _border = Color(0xFF21262D);
const _text = Color(0xFFE6EDF3);
const _textDim = Color(0xFF7D8590);
const _bubbleOut = Color(0xFF0D2847);
const _bubbleIn = Color(0xFF1C2128);
const _errorBg = Color(0xFF3D0F0F);
const _errorFg = Color(0xFFFF7B72);

// ── theme ───────────────────────────────────────────────────────────────────

ThemeData _theme() => ThemeData(
      useMaterial3: true,
      brightness: Brightness.dark,
      colorScheme: const ColorScheme.dark(
        primary: _teal,
        surface: _surface,
        error: _errorFg,
      ),
      scaffoldBackgroundColor: _bg,
      dividerColor: _border,
      hintColor: _textDim,
      appBarTheme: const AppBarTheme(
        backgroundColor: _surface,
        elevation: 0,
        centerTitle: false,
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: _surface2,
        border: _inputBorder(_border),
        enabledBorder: _inputBorder(_border),
        focusedBorder: _inputBorder(_teal),
        hintStyle: const TextStyle(color: _textDim, fontSize: 14),
        contentPadding:
            const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: _teal,
          foregroundColor: _bg,
          padding: const EdgeInsets.all(14),
          shape:
              RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        ),
      ),
      listTileTheme: const ListTileThemeData(
        selectedTileColor: Color(0xFF0D1F2D),
        selectedColor: _teal,
        iconColor: _textDim,
        textColor: _text,
        dense: true,
        contentPadding: EdgeInsets.symmetric(horizontal: 14, vertical: 4),
      ),
      iconButtonTheme: IconButtonThemeData(
        style: IconButton.styleFrom(foregroundColor: _textDim),
      ),
      textTheme: const TextTheme(
        titleLarge: TextStyle(
          color: _text,
          fontSize: 15,
          fontWeight: FontWeight.w600,
          letterSpacing: 0.1,
        ),
        bodyLarge: TextStyle(color: _text, fontSize: 14, height: 1.4),
        bodyMedium: TextStyle(color: _text, fontSize: 13, height: 1.4),
        bodySmall: TextStyle(color: _textDim, fontSize: 11),
      ),
    );

InputBorder _inputBorder(Color c) => OutlineInputBorder(
      borderRadius: BorderRadius.circular(12),
      borderSide: BorderSide(color: c),
    );

// ── app ─────────────────────────────────────────────────────────────────────

void main() {
  runApp(const SidebandApp());
}

class SidebandApp extends StatelessWidget {
  const SidebandApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Sideband',
      theme: _theme(),
      home: const _ChatScreen(),
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
  });
  final String name;
  final String onion;
  final String pubkey;
  final String x25519Pubkey;
  final bool ratchetActive;

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
    if (ratchetActive) return 'Double Ratchet';
    if (staticKeyActive) return 'Static key';
    return 'Signed only';
  }

  String get securityDescription {
    if (ratchetActive) {
      return 'Double Ratchet active: encrypted with forward secrecy.';
    }
    if (staticKeyActive) {
      return 'Static X25519 encryption: encrypted, but no ratchet yet.';
    }
    return 'Signed-only legacy contact: no X25519 encryption key is present.';
  }
}

class ChatMsg {
  const ChatMsg({
    required this.id,
    required this.direction,
    required this.status,
    required this.contact,
    required this.text,
    required this.tsMs,
  });

  final int id;
  final String direction;
  final String status;
  final String contact;
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

class ShareInfo {
  const ShareInfo({required this.command, required this.qr});
  final String command;
  final List<String> qr;
}

class _SendMessageIntent extends Intent {
  const _SendMessageIntent();
}

// ── cli ─────────────────────────────────────────────────────────────────────

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

  bool _ratchetActive(String contactName) {
    if (contactName.trim().isEmpty) return false;
    return File('${expandedProfilePath()}/ratchet/$contactName.bin').existsSync();
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
    final raw = await _run(['share', '--profile', profile, '--onion', onion, '--json']);
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

  Future<String> clearHistory({String? contact}) {
    final args = ['history', '--profile', profile, '--clear'];
    if (contact != null && contact.trim().isNotEmpty) {
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
      ));
    }
    parsed.sort((a, b) => a.name.compareTo(b.name));
    return parsed;
  }

  Future<_History> history({String? contact, int limit = 80}) async {
    final args = ['history', '--profile', profile, '--limit', '$limit', '--json'];
    if (contact != null && contact.trim().isNotEmpty) {
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
      parsed.add(ChatMsg(
        id: (item['id'] as num).toInt(),
        direction: item['direction'] as String,
        status: _statusLabel((item['status'] as num).toInt()),
        contact: item['contact'] as String,
        text: item['body'] as String,
        tsMs: (item['timestamp_ms'] as num).toInt(),
      ));
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
    await _run(['send', '--profile', profile, '--to', to, '--message', message]);
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
  bool shouldRepaint(covariant _QrPainter oldDelegate) => oldDelegate.rows != rows;
}

// ── screen ──────────────────────────────────────────────────────────────────

class _ChatScreen extends StatefulWidget {
  const _ChatScreen();

  @override
  State<_ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<_ChatScreen> {
  final _cli = _Cli();
  final _input = TextEditingController();
  final _scroll = ScrollController();

  List<Contact> _contacts = [];
  List<ChatMsg> _msgs = [];
  final List<ChatMsg> _pendingMsgs = [];
  Contact? _sel;
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
  late final Timer _poll;

  @override
  void initState() {
    super.initState();
    _listenerLogFile = File('${_cli.profile}/gui-listener.log');
    _startListener();
    _load();
    _poll = Timer.periodic(const Duration(seconds: 6), (_) => _refresh());
  }

  @override
  void dispose() {
    _poll.cancel();
    _listener?.kill(ProcessSignal.sigterm);
    _input.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _appendListenerLog(String stream, String chunk) async {
    final text = chunk.trim();
    if (text.isEmpty) return;
    final line = '[${DateTime.now().toIso8601String()}] $stream: $text\n';
    _listenerLogTail = (_listenerLogTail + line);
    if (_listenerLogTail.length > 4000) {
      _listenerLogTail = _listenerLogTail.substring(_listenerLogTail.length - 4000);
    }
    try {
      await _listenerLogFile.parent.create(recursive: true);
      await _listenerLogFile.writeAsString(line, mode: FileMode.append, flush: true);
    } catch (_) {
      // The visible UI error matters more than failing to write diagnostics.
    }
  }

  String _expandedProfilePath() {
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
            if ((lower.contains('send error') ||
                    lower.contains('resolve error') ||
                    lower.contains('control error')) &&
                !_isRecentSendTransient(lower)) {
              _error = msg;
            }
          });
          if (lower.contains('message received') ||
              lower.contains('incoming connection') ||
              lower.contains('message sent') ||
              lower.contains('send error') ||
              lower.contains('resolve error')) {
            unawaited(_refresh());
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
    return stored.out &&
        stored.contact == pending.contact &&
        stored.text == pending.text &&
        (stored.tsMs - pending.tsMs).abs() <= 120000;
  }

  List<ChatMsg> _mergePending(List<ChatMsg> history) {
    _pendingMsgs.removeWhere(
        (pending) => history.any((stored) => _matchesPending(pending, stored)));
    final merged = [..._pendingMsgs, ...history];
    merged.sort((a, b) => a.tsMs.compareTo(b.tsMs));
    return merged;
  }

  Color _securityColor(Contact c) => c.ratchetActive
      ? _teal
      : c.staticKeyActive
          ? const Color(0xFF9CDCFE)
          : _textDim;

  IconData _securityIcon(Contact c) => c.ratchetActive
      ? Icons.lock_rounded
      : c.staticKeyActive
          ? Icons.lock_outline
          : Icons.edit_outlined;

  Future<void> _sendViaListener({required String to, required String message}) async {
    final listener = _listener;
    if (listener == null) {
      throw Exception('listener control channel is not available; restart the GUI');
    }
    listener.stdin.writeln(jsonEncode({
      'cmd': 'send',
      'to': to,
      'message': message,
    }));
    await listener.stdin.flush();
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final c = await _cli.contacts();
      var s = _sel;
      if (s == null && c.isNotEmpty) {
        s = c.first;
      } else if (s != null) {
        final idx = c.indexWhere((x) => x.name == s!.name);
        s = idx >= 0 ? c[idx] : (c.isNotEmpty ? c.first : null);
      }
      final h = await _historyVisibleFor(s?.name);
      setState(() {
        _contacts = c;
        _sel = s;
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

  Future<_History> _historyVisibleFor(String? contact) async {
    final filtered = await _cli.history(contact: contact);
    if (filtered.msgs.isNotEmpty || contact == null || contact.trim().isEmpty) {
      return filtered;
    }

    // If inbound was stored under a raw pubkey/verified-peer because the local
    // contact record is stale, a strict contact filter hides the only evidence.
    // Fall back to the global transcript instead of showing a lying empty pane.
    return _cli.history();
  }

  Future<void> _refresh() async {
    if (_sel == null) return;
    try {
      final h = await _historyVisibleFor(_sel!.name);
      setState(() {
        _msgs = _mergePending(h.msgs);
      });
      _scrollToBottom();
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = '$e');
    }
  }

  Future<void> _send() async {
    final c = _sel;
    final t = _input.text.trim();
    if (t.isEmpty) return;
    _input.clear();

    if (t.startsWith('/')) {
      await _runSlashCommand(t);
      return;
    }

    if (c == null) return;

    // optimistic
    final now = DateTime.now();
    final pending = ChatMsg(
        id: -now.millisecondsSinceEpoch,
        direction: 'out',
        status: 'sending',
        contact: c.name,
        text: t,
        tsMs: now.millisecondsSinceEpoch);
    setState(() {
      _sending = true;
      _lastSendStartedAt = now;
      _error = null;
      _pendingMsgs.add(pending);
      _msgs = _mergePending(_msgs.where((m) => !m.sending).toList());
    });
    _scrollToBottom();

    try {
      await _sendViaListener(to: c.name, message: t);
      await _refresh();
    } catch (e) {
      _pendingMsgs.removeWhere((m) => m.id == pending.id);
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
        backgroundColor: _surface,
        title: Text(title, style: const TextStyle(color: _text)),
        content: SingleChildScrollView(
          child: SelectableText(body,
              style: const TextStyle(color: _text, fontSize: 12, height: 1.35)),
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
            backgroundColor: _surface,
            title: Text(title, style: const TextStyle(color: _text)),
            content: Text(body, style: const TextStyle(color: _textDim)),
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
              '/send <contact> <msg>\n/history [contact]\n/contacts\n/add <name> <onion> <ed25519_pk> <x25519_pk>\n/delete <contact>\n/name [display-name]\n/whoami\n/share\n/onion\n/ratchet <contact>\n/status\n/clear\n/clearhistory [contact]\n/settings');
          return;
        case 'send':
          if (parts.length < 3) throw Exception('usage: /send <contact> <message>');
          final contact = parts[1];
          final msg = raw.split(RegExp(r'\s+')).skip(2).join(' ');
          await _cli.send(to: contact, message: msg);
          await _load();
          return;
        case 'history':
          final contact = parts.length > 1 ? parts[1] : null;
          final h = await _cli.history(contact: contact, limit: 200);
          final lines = h.msgs.reversed.map((m) {
            final arrow = m.direction == 'out' ? '→' : '←';
            return '${_hm(m.ts)} $arrow ${m.contact}: ${m.text}';
          });
          _showInfo('History${contact == null ? '' : ' for $contact'}',
              lines.isEmpty ? '(no messages)' : lines.join('\n'));
          return;
        case 'contacts':
          await _load();
          _showInfo(
              'Contacts',
              _contacts.isEmpty
                  ? '(no contacts)'
                  : _contacts
                      .map((c) =>
                          '${c.name}\nsecurity=${c.securityLabel}\nonion=${c.onion}\npubkey=${c.pubkey}\nx25519=${c.x25519Pubkey}')
                      .join('\n\n'));
          return;
        case 'add':
          if (parts.length < 5) {
            throw Exception('usage: /add <name> <onion> <ed25519_pubkey_b64> <x25519_pubkey_b64>');
          }
          await _cli.addContact(
            name: parts[1],
            onion: parts[2],
            pubkey: parts[3],
            x25519Pubkey: parts[4],
          );
          await _load();
          return;
        case 'delete':
          if (parts.length < 2) throw Exception('usage: /delete <contact>');
          _showInfo('Contact deleted', await _cli.deleteContact(parts[1]));
          await _load();
          return;
        case 'name':
          _showInfo('Name', await _cli.name(arg.isEmpty ? null : arg));
          return;
        case 'whoami':
          _showInfo('Identity', await _cli.identity());
          return;
        case 'share':
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
          final onion = _listenerStatus.startsWith('listening ')
              ? _listenerStatus.substring('listening '.length)
              : '(waiting for Tor)';
          _showInfo('Share contact', '/add $name $onion $ed $x');
          return;
        case 'onion':
          _showInfo('Onion', _listenerStatus.startsWith('listening ')
              ? _listenerStatus.substring('listening '.length)
              : '(waiting for Tor)');
          return;
        case 'ratchet':
          if (parts.length < 2) throw Exception('usage: /ratchet <contact>');
          final result = await _cli.ratchet(parts[1]);
          await _load();
          _showInfo('Ratchet', result);
          return;
        case 'status':
          _showInfo('Status',
              'listener: $_listenerStatus\nprofile: ${_cli.profile}\nbinary: ${_cli.bin}\ncontacts: ${_contacts.length}\nmessages visible: ${_msgs.length}');
          return;
        case 'clear':
          setState(() => _msgs = []);
          return;
        case 'clearhistory':
          final contact = parts.length > 1 ? parts[1] : _sel?.name;
          final clearTarget = contact == null ? 'all message history' : 'history for $contact';
          if (!await _confirm('Clear history', 'Delete $clearTarget?')) return;
          _showInfo('History cleared', await _cli.clearHistory(contact: contact));
          await _load();
          return;
        case 'settings':
          await _showSettings();
          return;
        case 'file':
        case 'transfers':
          throw Exception('/$cmd is not wired in the GUI yet. Backend support is TUI-only right now. Annoying, but honest.');
        default:
          throw Exception('unknown command: /$cmd (try /help)');
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _showAddContactDialog() => _showContactDialog();

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
          backgroundColor: _surface,
          title: Text(editing ? 'Edit contact' : 'Add contact',
              style: const TextStyle(color: _text)),
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

      final newName = name.text.trim();
      if (newName.isEmpty) throw Exception('contact name is required');
      await _cli.addContact(
        name: newName,
        onion: onion.text.trim(),
        pubkey: pubkey.text.trim(),
        x25519Pubkey: x25519.text.trim(),
      );
      if (editing && contact.name != newName) {
        await _cli.deleteContact(contact.name);
      }
      await _load();
      if (editing) {
        for (final updated in _contacts.where((c) => c.name == newName)) {
          setState(() => _sel = updated);
          break;
        }
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      name.dispose();
      onion.dispose();
      pubkey.dispose();
      x25519.dispose();
    }
  }

  Future<void> _deleteContact(Contact contact) async {
    if (!await _confirm(
        'Delete contact', 'Delete ${contact.name}? Message history is kept.')) {
      return;
    }
    try {
      _showInfo('Contact deleted', await _cli.deleteContact(contact.name));
      await _load();
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
      _showInfo('History deleted', await _cli.clearHistory(contact: contact.name));
      await _load();
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _showContactMenu(Contact contact, Offset position) async {
    final action = await showMenu<String>(
      context: context,
      position:
          RelativeRect.fromLTRB(position.dx, position.dy, position.dx, position.dy),
      color: _surface,
      items: const [
        PopupMenuItem(value: 'history', child: Text('Show history')),
        PopupMenuItem(value: 'clear-history', child: Text('Delete history')),
        PopupMenuDivider(),
        PopupMenuItem(value: 'edit', child: Text('Edit contact')),
        PopupMenuItem(value: 'delete', child: Text('Delete contact')),
        PopupMenuDivider(),
        PopupMenuItem(value: 'details', child: Text('Contact details')),
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
    final controller = TextEditingController(text: current.trim());
    try {
      final ok = await showDialog<bool>(
        context: context,
        builder: (_) => AlertDialog(
          backgroundColor: _surface,
          title: const Text('Display name', style: TextStyle(color: _text)),
          content: TextField(
            controller: controller,
            decoration: const InputDecoration(labelText: 'Name'),
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
      );
      if (ok == true) {
        _showInfo('Name', await _cli.name(controller.text.trim()));
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    } finally {
      controller.dispose();
    }
  }

  String? _currentOnion() {
    final onion = _listenerOnion;
    if (onion == null || onion.trim().isEmpty) return null;
    return onion.trim();
  }

  Future<ShareInfo> _shareInfo() async {
    final onion = _currentOnion();
    if (onion == null) {
      throw Exception('onion address is not ready yet');
    }
    return _cli.share(onion);
  }

  Future<void> _showShareDialog() async {
    try {
      final share = await _shareInfo();
      if (!mounted) return;
      await showDialog<void>(
        context: context,
        builder: (_) => AlertDialog(
          backgroundColor: _surface,
          title: const Text('Share contact', style: TextStyle(color: _text)),
          content: SizedBox(
            width: 520,
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
                    width: 280,
                    height: 280,
                    child: CustomPaint(painter: _QrPainter(share.qr)),
                  ),
                ),
                const SizedBox(height: 16),
                const Text(
                  'Scan this QR code to add this contact, or copy the command below.',
                  textAlign: TextAlign.center,
                  style: TextStyle(color: _textDim),
                ),
                const SizedBox(height: 12),
                SelectableText(
                  share.command,
                  style: const TextStyle(
                    color: _text,
                    fontFamily: 'monospace',
                    fontSize: 12,
                  ),
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Close'),
            ),
            FilledButton.icon(
              icon: const Icon(Icons.copy, size: 16),
              label: const Text('Copy'),
              onPressed: () async {
                await Clipboard.setData(ClipboardData(text: share.command));
                if (context.mounted) Navigator.pop(context);
                _showInfo('Share contact', '${share.command}\n\nCopied to clipboard.');
              },
            ),
          ],
        ),
      );
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _clearAllHistory() async {
    if (!await _confirm('Delete all history', 'Delete all message history?')) return;
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
      builder: (_) => AlertDialog(
        backgroundColor: _surface,
        title: const Text('Settings', style: TextStyle(color: _text)),
        content: SizedBox(
          width: 560,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(Icons.badge_outlined),
                title: const Text('Display name'),
                subtitle: const Text('Set the name shared with contacts'),
                onTap: () {
                  Navigator.pop(context);
                  unawaited(_changeDisplayName());
                },
              ),
              ListTile(
                leading: const Icon(Icons.person_add_alt_1),
                title: const Text('Add contact'),
                subtitle: const Text('Paste a shared /add command by hand'),
                onTap: () {
                  Navigator.pop(context);
                  unawaited(_showAddContactDialog());
                },
              ),
              ListTile(
                leading: const Icon(Icons.ios_share),
                title: const Text('Share my contact'),
                subtitle: Text(_listenerStatus),
                onTap: () {
                  Navigator.pop(context);
                  unawaited(_showShareDialog());
                },
              ),
              ListTile(
                leading: const Icon(Icons.fingerprint),
                title: const Text('Show identity'),
                subtitle: const Text('Public keys and profile identity'),
                onTap: () {
                  Navigator.pop(context);
                  unawaited(_runSlashCommand('/whoami'));
                },
              ),
              ListTile(
                leading: const Icon(Icons.info_outline),
                title: const Text('Runtime status'),
                subtitle: Text('${_cli.profile} • ${_contacts.length} contacts'),
                onTap: () {
                  Navigator.pop(context);
                  unawaited(_runSlashCommand('/status'));
                },
              ),
              ListTile(
                leading: const Icon(Icons.delete_sweep_outlined),
                title: const Text('Delete all history'),
                subtitle: const Text('Contacts stay. Messages go away.'),
                onTap: () {
                  Navigator.pop(context);
                  unawaited(_clearAllHistory());
                },
              ),
              if (!_listenerRunning)
                ListTile(
                  leading: const Icon(Icons.power_settings_new),
                  title: const Text('Start listener'),
                  subtitle: const Text('Bring the onion service back up'),
                  onTap: () {
                    Navigator.pop(context);
                    unawaited(_startListener());
                  },
                ),
            ],
          ),
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

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: LayoutBuilder(
        builder: (context, constraints) {
          // GTK can hand Flutter a 1x1 surface before the first real frame.
          // Rendering the full layout there just trips Flex overflow asserts.
          if (constraints.maxWidth < 80 || constraints.maxHeight < 80) {
            return const ColoredBox(color: _bg);
          }

          if (_loading) {
            return Center(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const SizedBox(
                    width: 28,
                    height: 28,
                    child: CircularProgressIndicator(
                        strokeWidth: 2.5, color: _teal),
                  ),
                  const SizedBox(height: 16),
                  Text('Connecting…',
                      style: Theme.of(context)
                          .textTheme
                          .bodyMedium
                          ?.copyWith(color: _textDim)),
                ],
              ),
            );
          }

          if (constraints.maxWidth < 720) {
            return _sel == null ? _sidebar() : _chat();
          }

          return Row(
            children: [
              SizedBox(width: 260, child: _sidebar()),
              Container(width: 1, color: _border),
              Expanded(child: _sel == null ? _empty() : _chat()),
            ],
          );
        },
      ),
    );
  }

  // ── sidebar ──────────────────────────────────────────────────────────────

  Widget _sidebar() {
    return Container(
      color: _surface,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // header
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 14, 14, 10),
            child: Row(
              children: [
                const Text(
                  'Messages',
                  style: TextStyle(
                    color: _text,
                    fontSize: 17,
                    fontWeight: FontWeight.w700,
                    letterSpacing: 0.2,
                  ),
                ),
                const Spacer(),
                SizedBox(
                  width: 34,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints.tightFor(width: 34, height: 34),
                    icon: const Icon(Icons.person_add_alt_1, size: 19),
                    onPressed: _showAddContactDialog,
                    tooltip: 'Add contact',
                  ),
                ),
                SizedBox(
                  width: 34,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints.tightFor(width: 34, height: 34),
                    icon: const Icon(Icons.qr_code, size: 19),
                    onPressed: _showShareDialog,
                    tooltip: 'Share contact',
                  ),
                ),
                SizedBox(
                  width: 34,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints.tightFor(width: 34, height: 34),
                    icon: const Icon(Icons.settings, size: 19),
                    onPressed: _showSettings,
                    tooltip: 'Settings',
                  ),
                ),
                SizedBox(
                  width: 34,
                  child: IconButton(
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints.tightFor(width: 34, height: 34),
                    icon: const Icon(Icons.refresh, size: 19),
                    onPressed: _load,
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
                : _contacts.isEmpty
                    ? Center(
                        child: Padding(
                          padding: const EdgeInsets.all(24),
                          child: Text(
                            'No contacts yet.\nUse + or /add <name> <onion> <ed25519> <x25519>.',
                            textAlign: TextAlign.center,
                            style: TextStyle(
                                color: _textDim, fontSize: 12, height: 1.6),
                          ),
                        ),
                      )
                    : ListView.builder(
                    padding: const EdgeInsets.symmetric(horizontal: 6),
                    itemCount: _contacts.length,
                    itemBuilder: (_, i) {
                      final c = _contacts[i];
                      final on = _sel?.name == c.name;
                      return GestureDetector(
                        onSecondaryTapDown: (details) =>
                            _showContactMenu(c, details.globalPosition),
                        child: ListTile(
                          selected: on,
                          leading: CircleAvatar(
                            radius: 17,
                            backgroundColor: c.avatarColor,
                            child: Text(c.initial,
                                style: const TextStyle(
                                    color: Colors.white,
                                    fontSize: 12,
                                    fontWeight: FontWeight.w700)),
                          ),
                          title: Row(
                            children: [
                              Expanded(
                                child: Text(c.name,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: TextStyle(
                                      fontSize: 13.5,
                                      fontWeight:
                                          on ? FontWeight.w600 : FontWeight.w500,
                                      color: on ? _teal : _text,
                                    )),
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
                              Text(
                                c.shortOnion,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(fontSize: 10.5, color: _textDim),
                              ),
                              const SizedBox(height: 1),
                              Row(
                                children: [
                                  Icon(_securityIcon(c),
                                      size: 9, color: _securityColor(c)),
                                  const SizedBox(width: 3),
                                  Expanded(
                                    child: Text(c.securityLabel,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: TextStyle(
                                            fontSize: 10,
                                            color: _securityColor(c))),
                                  ),
                                ],
                              ),
                            ],
                          ),
                          trailing: PopupMenuButton<String>(
                            tooltip: 'Contact menu',
                            icon: const Icon(Icons.more_vert, size: 17),
                            color: _surface,
                            onSelected: (action) async {
                              switch (action) {
                                case 'history':
                                  await _runSlashCommand('/history ${c.name}');
                                  return;
                                case 'clear-history':
                                  await _clearHistoryFor(c);
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
                            itemBuilder: (_) => const [
                              PopupMenuItem(
                                  value: 'history', child: Text('Show history')),
                              PopupMenuItem(
                                  value: 'clear-history',
                                  child: Text('Delete history')),
                              PopupMenuDivider(),
                              PopupMenuItem(
                                  value: 'edit', child: Text('Edit contact')),
                              PopupMenuItem(
                                  value: 'delete', child: Text('Delete contact')),
                              PopupMenuDivider(),
                              PopupMenuItem(
                                  value: 'details', child: Text('Contact details')),
                            ],
                          ),
                          onTap: () async {
                            setState(() => _sel = c);
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
            decoration: const BoxDecoration(
              border: Border(top: BorderSide(color: _border, width: 1)),
            ),
            child: Row(
              children: [
                Container(
                  width: 7,
                  height: 7,
                  decoration: BoxDecoration(
                      color: _listenerRunning ? _teal : _errorFg,
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
                            color: _listenerRunning ? _teal : _errorFg),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                      Text(
                        _cli.profile,
                        style: const TextStyle(fontSize: 10, color: _textDim),
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
                    onPressed: _startListener,
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
          color: _errorBg,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: _errorFg.withAlpha(90)),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.warning_amber_rounded, size: 16, color: _errorFg),
                const SizedBox(width: 6),
                Text('Backend error',
                    style: TextStyle(
                        color: _errorFg,
                        fontSize: 12,
                        fontWeight: FontWeight.w700)),
              ],
            ),
            const SizedBox(height: 8),
            Text(_error!, style: TextStyle(color: _errorFg, fontSize: 11.5)),
            const SizedBox(height: 12),
            OutlinedButton.icon(
              onPressed: _load,
              icon: const Icon(Icons.refresh, size: 14),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }

  Widget _empty() {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.send_rounded, size: 42, color: _textDim.withAlpha(50)),
          const SizedBox(height: 14),
          Text('Select a contact',
              style: TextStyle(color: _textDim, fontSize: 14)),
        ],
      ),
    );
  }

  // ── chat ─────────────────────────────────────────────────────────────────

  Widget _chat() {
    return Container(
      color: _bg,
      child: Column(
        children: [
          _chatHeader(),
          Container(height: 1, color: _border),
          if (_error != null) _errorBanner(),
          Expanded(child: _msgList()),
          Container(height: 1, color: _border),
          _inputArea(),
        ],
      ),
    );
  }

  Widget _chatHeader() {
    final c = _sel!;
    return Container(
      color: _surface,
      padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
      child: Row(
        children: [
          CircleAvatar(
            radius: 16,
            backgroundColor: c.avatarColor,
            child: Text(c.initial,
                style: const TextStyle(
                    color: Colors.white,
                    fontSize: 11,
                    fontWeight: FontWeight.w700)),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(c.name,
                    style: const TextStyle(
                        color: _text,
                        fontSize: 14.5,
                        fontWeight: FontWeight.w700)),
                const SizedBox(height: 1),
                Tooltip(
                  message: c.securityDescription,
                  child: Row(
                    children: [
                      Icon(_securityIcon(c), size: 11, color: _securityColor(c)),
                      const SizedBox(width: 4),
                      Text(c.securityLabel,
                          style: TextStyle(
                              fontSize: 10.5, color: _securityColor(c))),
                    ],
                  ),
                ),
              ],
            ),
          ),
          IconButton(
            icon: const Icon(Icons.history, size: 18),
            tooltip: 'History',
            onPressed: () => _runSlashCommand('/history ${c.name}'),
          ),
          IconButton(
            icon: const Icon(Icons.delete_sweep_outlined, size: 18),
            tooltip: 'Delete history',
            onPressed: () => _clearHistoryFor(c),
          ),
          Tooltip(
            message: _listenerStatus,
            child: Container(
              width: 7,
              height: 7,
              decoration: BoxDecoration(
                color: _listenerRunning ? _teal : _errorFg,
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
        color: _errorBg,
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
        child: Row(
          children: [
            Icon(Icons.error_outline, size: 14, color: _errorFg),
            const SizedBox(width: 6),
            Expanded(
              child: Text(_error!,
                  style: TextStyle(color: _errorFg, fontSize: 11.5)),
            ),
            Icon(Icons.close, size: 13, color: _errorFg),
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
              style: TextStyle(color: _textDim, fontSize: 13)));
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
        return Column(
          children: [
            if (showDate) _dateLabel(m.ts),
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
          Expanded(child: Container(height: 1, color: _border)),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Text(label,
                style: const TextStyle(
                    fontSize: 10.5,
                    color: _textDim,
                    fontWeight: FontWeight.w600,
                    letterSpacing: 0.3)),
          ),
          Expanded(child: Container(height: 1, color: _border)),
        ],
      ),
    );
  }

  Widget _bubble(ChatMsg m) {
    final right = m.out;
    return Align(
      alignment: right ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        constraints:
            BoxConstraints(maxWidth: MediaQuery.of(context).size.width * 0.65),
        padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 9),
        margin: const EdgeInsets.only(bottom: 1),
        decoration: BoxDecoration(
          color: right ? _bubbleOut : _bubbleIn,
          borderRadius: BorderRadius.only(
            topLeft: const Radius.circular(14),
            topRight: const Radius.circular(14),
            bottomLeft: Radius.circular(right ? 14 : 3),
            bottomRight: Radius.circular(right ? 3 : 14),
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(m.text,
                style:
                    const TextStyle(color: _text, fontSize: 14, height: 1.35)),
            const SizedBox(height: 3),
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(_hm(m.ts),
                    style: TextStyle(
                        fontSize: 10, color: _textDim.withAlpha(153))),
                if (right) ...[
                  const SizedBox(width: 4),
                  _statusIcon(m),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _statusIcon(ChatMsg m) {
    if (m.sending) {
      return SizedBox(
        width: 9,
        height: 9,
        child: CircularProgressIndicator(
            strokeWidth: 1.5, color: _textDim.withAlpha(120)),
      );
    }
    if (m.failed) {
      return const Icon(Icons.error_outline, size: 12, color: _errorFg);
    }
    if (m.status == 'delivered') {
      return Icon(Icons.done_all, size: 13, color: _teal.withAlpha(180));
    }
    return Icon(Icons.done, size: 13, color: _teal.withAlpha(160));
  }

  // ── input ────────────────────────────────────────────────────────────────

  Widget _inputArea() {
    return Container(
      color: _surface,
      padding: const EdgeInsets.fromLTRB(14, 10, 14, 12),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Expanded(
            child: Shortcuts(
              shortcuts: const <ShortcutActivator, Intent>{
                SingleActivator(LogicalKeyboardKey.enter): _SendMessageIntent(),
              },
              child: Actions(
                actions: <Type, Action<Intent>>{
                  _SendMessageIntent: CallbackAction<_SendMessageIntent>(
                    onInvoke: (_) {
                      if (!_sending) unawaited(_send());
                      return null;
                    },
                  ),
                },
                child: TextField(
                  controller: _input,
                  enabled: !_sending,
                  minLines: 1,
                  maxLines: 4,
                  keyboardType: TextInputType.multiline,
                  style: const TextStyle(fontSize: 14, color: _text),
                  decoration: const InputDecoration(
                      hintText: 'Message or /help for commands…'),
                  textInputAction: TextInputAction.newline,
                ),
              ),
            ),
          ),
          const SizedBox(width: 8),
          FilledButton(
            onPressed: _sending ? null : _send,
            style: FilledButton.styleFrom(
              minimumSize: const Size(42, 42),
              padding: EdgeInsets.zero,
            ),
            child: _sending
                ? SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(
                        strokeWidth: 2, color: _bg.withAlpha(140)),
                  )
                : const Icon(Icons.send_rounded, size: 17),
          ),
        ],
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
