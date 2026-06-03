import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';

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
  const Contact({required this.name, required this.onion});
  final String name;
  final String onion;

  String get initial => name.isNotEmpty ? name[0].toUpperCase() : '?';

  Color get avatarColor {
    final h = name.codeUnits.fold<int>(0, (a, b) => a + b) % 360;
    return HSLColor.fromAHSL(1, h.toDouble(), 0.45, 0.42).toColor();
  }

  String get shortOnion {
    if (onion.length <= 20) return onion;
    return '${onion.substring(0, 10)}…${onion.substring(onion.length - 8)}';
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
  bool get delivered => status == 'delivered' || status == 'sent';
}

class _History {
  const _History({required this.msgs, required this.maxId, required this.bin});
  final List<ChatMsg> msgs;
  final int? maxId;
  final String bin;
}

// ── cli ─────────────────────────────────────────────────────────────────────

class _Cli {
  _Cli();

  static String _defaultBin() {
    final env = Platform.environment['SIDEBAND_BIN'];
    if (env != null && env.trim().isNotEmpty) return env;

    const candidates = [
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
    await _run(['send', '--profile', profile, '--to', to, '--message', message, '--static']);
  }
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
  Contact? _sel;
  Process? _listener;
  bool _listenerRunning = false;
  String _listenerStatus = 'listener stopped';
  String _listenerLogTail = '';
  late final File _listenerLogFile;
  bool _loading = true;
  bool _sending = false;
  String? _error;
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

  Future<void> _startListener() async {
    if (_listener != null) return;
    setState(() {
      _listenerStatus = 'listener starting';
      _listenerRunning = false;
    });

    try {
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
          final last = msg.split('\n').last.trim();
          setState(() {
            _listenerStatus = last.startsWith('onion=')
                ? 'listening ${last.substring('onion='.length)}'
                : last;
            if (last.startsWith('onion=')) {
              _listenerRunning = true;
            }
          });
          final lower = msg.toLowerCase();
          if (lower.contains('message received') ||
              lower.contains('incoming connection')) {
            unawaited(_refresh());
          }
        }
      });
      p.stderr.transform(systemEncoding.decoder).listen((chunk) {
        unawaited(_appendListenerLog('stderr', chunk));
        final msg = chunk.trim();
        if (msg.isNotEmpty && mounted) {
          setState(() {
            _listenerStatus = msg.split('\n').last.trim();
            // Keep the full backend failure visible somewhere. Silent empty
            // panes are how we got here.
            if (msg.toLowerCase().contains('error') ||
                msg.toLowerCase().contains('failed')) {
              _error = msg;
            }
          });
          final lower = msg.toLowerCase();
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
        _listenerStatus = 'listener failed';
        _error = '$e';
      });
    }
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
        _msgs = h.msgs;
        _loading = false;
      });
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
        _msgs = h.msgs;
        _error = null;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = '$e');
    }
  }

  Future<void> _send() async {
    final c = _sel;
    if (c == null) return;
    final t = _input.text.trim();
    if (t.isEmpty) return;
    _input.clear();

    // optimistic
    final now = DateTime.now();
    setState(() {
      _sending = true;
      _msgs = [
        ChatMsg(
            id: -now.millisecondsSinceEpoch,
            direction: 'out',
            status: 'sending',
            contact: c.name,
            text: t,
            tsMs: now.millisecondsSinceEpoch),
        ..._msgs,
      ];
    });

    try {
      await _cli.send(to: c.name, message: t);
      await _refresh();
    } catch (e) {
      setState(() => _error = '$e');
      await _refresh();
    } finally {
      if (mounted) setState(() => _sending = false);
    }
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
                IconButton(
                  icon: const Icon(Icons.refresh, size: 19),
                  onPressed: _load,
                  tooltip: 'Refresh',
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
                            'No contacts yet.\nsideband contact add …',
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
                      return ListTile(
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
                        title: Text(c.name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: 13.5,
                              fontWeight:
                                  on ? FontWeight.w600 : FontWeight.w500,
                              color: on ? _teal : _text,
                            )),
                        subtitle: Text(
                          c.shortOnion,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(fontSize: 10.5, color: _textDim),
                        ),
                        onTap: () async {
                          setState(() => _sel = c);
                          await _refresh();
                        },
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
                Row(
                  children: [
                    Icon(Icons.lock, size: 9, color: _textDim),
                    const SizedBox(width: 3),
                    Text('End-to-end encrypted',
                        style: TextStyle(fontSize: 10.5, color: _textDim)),
                  ],
                ),
              ],
            ),
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
            child: TextField(
              controller: _input,
              enabled: !_sending,
              minLines: 1,
              maxLines: 4,
              style: const TextStyle(fontSize: 14, color: _text),
              decoration: const InputDecoration(hintText: 'Send a message…'),
              onSubmitted: (_) => _sending ? null : _send(),
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
