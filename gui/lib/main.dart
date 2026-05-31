import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

// ── Theme ──────────────────────────────────────────────────────────────────

const _accentTeal = Color(0xFF26D9C8);
const _surfaceDark = Color(0xFF111418);
const _panelDark = Color(0xFF1A1D24);
const _chatBgDark = Color(0xFF0B0E14);
const _bubbleOut = Color(0xFF1E3A40);
const _bubbleIn = Color(0xFF1C1F26);
const _textPrimary = Color(0xFFE4E6EB);
const _textMuted = Color(0xFF8B95A6);
const _borderColor = Color(0xFF2A2D36);

ThemeData _buildTheme() => ThemeData(
      useMaterial3: true,
      brightness: Brightness.dark,
      colorScheme: const ColorScheme.dark(
        primary: _accentTeal,
        surface: _surfaceDark,
      ),
      scaffoldBackgroundColor: _chatBgDark,
      dividerColor: _borderColor,
      appBarTheme: const AppBarTheme(
        backgroundColor: _surfaceDark,
        foregroundColor: _textPrimary,
        elevation: 0,
        titleTextStyle: TextStyle(
          fontSize: 18,
          fontWeight: FontWeight.w700,
          color: _textPrimary,
          letterSpacing: 0.2,
        ),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: _accentTeal,
          foregroundColor: _chatBgDark,
          minimumSize: const Size(44, 44),
          padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 14),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(12),
          ),
        ),
      ),
      iconButtonTheme: IconButtonThemeData(
        style: IconButton.styleFrom(
          foregroundColor: _textMuted,
        ),
      ),
      listTileTheme: const ListTileThemeData(
        textColor: _textPrimary,
        iconColor: _textMuted,
        selectedTileColor: Color(0xFF1E2A30),
        selectedColor: _accentTeal,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.all(Radius.circular(10)),
        ),
        dense: true,
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: _panelDark,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: const BorderSide(color: _borderColor, width: 1),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: const BorderSide(color: _borderColor, width: 1),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: const BorderSide(color: _accentTeal, width: 1),
        ),
        contentPadding:
            const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        hintStyle: const TextStyle(color: _textMuted, fontSize: 14),
        labelStyle: const TextStyle(color: _textMuted, fontSize: 12),
      ),
      textTheme: const TextTheme(
        bodyLarge: TextStyle(color: _textPrimary, fontSize: 15),
        bodyMedium: TextStyle(color: _textPrimary, fontSize: 14),
        bodySmall: TextStyle(color: _textMuted, fontSize: 12),
      ),
      cardTheme: const CardThemeData(color: Colors.transparent, elevation: 0),
    );

// ── App entry ─────────────────────────────────────────────────────────────

void main() {
  runApp(const SidebandApp());
}

class SidebandApp extends StatelessWidget {
  const SidebandApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Sideband',
      theme: _buildTheme(),
      home: const ChatScreen(),
      debugShowCheckedModeBanner: false,
    );
  }
}

// ── Data models ───────────────────────────────────────────────────────────

class Contact {
  Contact({required this.name, required this.onion});

  final String name;
  final String onion;

  String get initials => name.isNotEmpty ? name[0].toUpperCase() : '?';

  Color get avatarColor {
    final hue = (name.hashCode % 360).abs().toDouble();
    return HSLColor.fromAHSL(1.0, hue, 0.55, 0.38).toColor();
  }
}

class ChatMessage {
  ChatMessage({
    required this.id,
    required this.direction,
    required this.status,
    required this.contact,
    required this.text,
    required this.timestampMs,
  });

  final int id;
  final String direction;
  final String status;
  final String contact;
  final String text;
  final int timestampMs;

  DateTime get timestamp => DateTime.fromMillisecondsSinceEpoch(timestampMs);

  bool get isOutgoing => direction.trim() == 'out';

  bool get delivered => status.trim() == 'delivered' || status.trim() == 'sent';

  bool get failed => status.trim() == 'failed';
}

class CliResult {
  CliResult({
    required this.executable,
    required this.stdout,
  });

  final String executable;
  final String stdout;
}

class HistoryResult {
  HistoryResult({
    required this.messages,
    required this.rawLineCount,
    required this.parsedLineCount,
    required this.maxId,
    required this.executable,
  });

  final List<ChatMessage> messages;
  final int rawLineCount;
  final int parsedLineCount;
  final int? maxId;
  final String executable;
}

// ── CLI bridge ────────────────────────────────────────────────────────────

class SidebandCli {
  SidebandCli();

  final String? _overridePath = Platform.environment['SIDEBAND_BIN'];
  final String profile =
      Platform.environment['SIDEBAND_PROFILE'] ?? '~/.sideband';

  Future<CliResult> _run(List<String> args) async {
    final executable = _resolveExecutable();
    final fullArgs = ['--profile', profile, ...args];
    final result = await Process.run(executable, fullArgs);
    if (result.exitCode != 0) {
      throw Exception((result.stderr as String).trim().isEmpty
          ? 'sideband failed with code ${result.exitCode}'
          : (result.stderr as String).trim());
    }
    return CliResult(
      executable: executable,
      stdout: (result.stdout as String).trim(),
    );
  }

  String _resolveExecutable() {
    final overridePath = _overridePath;
    if (overridePath != null && overridePath.isNotEmpty) {
      return overridePath;
    }
    return 'sideband';
  }

  Future<List<Contact>> contacts() async {
    final out = await _run(['contact', 'list']);
    final raw = out.stdout;
    if (raw.isEmpty) return [];
    return raw.split('\n').where((l) => l.trim().isNotEmpty).map((line) {
      final parts = line.split('\t');
      final name = parts.first.trim();
      final onion = parts
          .firstWhere(
            (p) => p.trim().startsWith('onion='),
            orElse: () => 'onion=unknown',
          )
          .split('=')
          .skip(1)
          .join('=')
          .trim();
      return Contact(name: name, onion: onion);
    }).toList();
  }

  Future<HistoryResult> history({String? contact, int limit = 80}) async {
    final args = ['history', '--limit', '$limit'];
    if (contact != null && contact.trim().isNotEmpty) {
      args.addAll(['--contact', contact.trim()]);
    }
    final out = await _run(args);
    final raw = out.stdout;
    if (raw.isEmpty) {
      return HistoryResult(
        messages: const [],
        rawLineCount: 0,
        parsedLineCount: 0,
        maxId: null,
        executable: out.executable,
      );
    }

    final pattern = RegExp(
      r'^\[(\d+)\]\s+(in|out)\s+(\S+)\s+(.+?)\s{2,}(.+)\s+ts=(\d+)$',
    );
    final parsed = <ChatMessage>[];
    final lines = raw.split('\n').where((l) => l.trim().isNotEmpty).toList();
    for (final line in lines) {
      final m = pattern.firstMatch(line.trim());
      if (m == null) continue;
      parsed.add(
        ChatMessage(
          id: int.parse(m.group(1)!),
          direction: m.group(2)!,
          status: m.group(3)!,
          contact: m.group(4)!.trim(),
          text: m.group(5)!,
          timestampMs: int.parse(m.group(6)!),
        ),
      );
    }

    final ordered = parsed.reversed.toList();
    final maxId = parsed.isEmpty
        ? null
        : parsed.map((m) => m.id).reduce((a, b) => a > b ? a : b);

    return HistoryResult(
      messages: ordered,
      rawLineCount: lines.length,
      parsedLineCount: parsed.length,
      maxId: maxId,
      executable: out.executable,
    );
  }

  Future<void> sendMessage(
      {required String to, required String message}) async {
    await _run(['send', '--to', to, '--message', message]);
  }
}

// ── Chat screen ───────────────────────────────────────────────────────────

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _cli = SidebandCli();
  final _inputController = TextEditingController();
  final _scrollController = ScrollController();

  List<Contact> _contacts = [];
  List<ChatMessage> _messages = [];
  Contact? _selectedContact;
  bool _loading = true;
  bool _sending = false;
  String? _error;
  Timer? _refreshTimer;
  DateTime? _lastRefreshAt;
  String _activeBinary = 'sideband';
  bool _showDebug = false;

  @override
  void initState() {
    super.initState();
    unawaited(_loadInitial());
    _refreshTimer = Timer.periodic(const Duration(seconds: 5), (_) {
      unawaited(_refreshMessages(silent: true));
    });
  }

  @override
  void dispose() {
    _refreshTimer?.cancel();
    _inputController.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  Future<void> _loadInitial() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final contacts = await _cli.contacts();
      var selected = _selectedContact;
      if (selected == null && contacts.isNotEmpty) {
        selected = contacts.first;
      } else if (selected != null) {
        final idx = contacts.indexWhere((c) => c.name == selected!.name);
        selected = idx >= 0
            ? contacts[idx]
            : (contacts.isNotEmpty ? contacts.first : null);
      }
      final historyResult = await _cli.history(contact: selected?.name);
      setState(() {
        _contacts = contacts;
        _selectedContact = selected;
        _messages = historyResult.messages;
        _lastRefreshAt = DateTime.now();
        _activeBinary = historyResult.executable;
        _loading = false;
      });
    } catch (e) {
      setState(() {
        _error = e.toString();
        _loading = false;
      });
    }
  }

  Future<void> _refreshMessages({bool silent = false}) async {
    if (_selectedContact == null) return;
    try {
      final hr = await _cli.history(contact: _selectedContact!.name);
      setState(() {
        _messages = hr.messages;
        _lastRefreshAt = DateTime.now();
        _activeBinary = hr.executable;
        if (!silent) _error = null;
      });
    } catch (e) {
      if (!silent) setState(() => _error = e.toString());
    }
  }

  Future<void> _send() async {
    final contact = _selectedContact;
    if (contact == null) return;
    final text = _inputController.text.trim();
    if (text.isEmpty) return;

    // Optimistic append
    final now = DateTime.now();
    final placeholder = ChatMessage(
      id: -now.millisecondsSinceEpoch,
      direction: 'out',
      status: 'sending',
      contact: contact.name,
      text: text,
      timestampMs: now.millisecondsSinceEpoch,
    );

    setState(() {
      _sending = true;
      _error = null;
      _messages = [..._messages, placeholder];
    });

    // Scroll to bottom
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scrollController.hasClients) {
        _scrollController.animateTo(
          0,
          duration: const Duration(milliseconds: 200),
          curve: Curves.easeOut,
        );
      }
    });

    try {
      await _cli.sendMessage(to: contact.name, message: text);
      _inputController.clear();
      await _refreshMessages();
    } catch (e) {
      setState(() => _error = e.toString());
      await _refreshMessages(silent: true);
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return Scaffold(
        backgroundColor: Colors.transparent,
        body: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox(
                width: 32,
                height: 32,
                child: CircularProgressIndicator(
                  strokeWidth: 2.5,
                  color: Theme.of(context).colorScheme.primary,
                ),
              ),
              const SizedBox(height: 16),
              Text(
                'Connecting…',
                style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                      color: _textMuted,
                    ),
              ),
            ],
          ),
        ),
      );
    }

    return Row(
      children: [
        // Sidebar
        Container(
          width: 280,
          color: _surfaceDark,
          child: _buildSidebar(),
        ),
        // Chat area
        Expanded(
          child: Container(
            color: _chatBgDark,
            child: _selectedContact == null
                ? _buildEmptyState()
                : _buildChatArea(),
          ),
        ),
      ],
    );
  }

  // ── Sidebar ─────────────────────────────────────────────────────────────

  Widget _buildSidebar() {
    return Column(
      children: [
        // Header
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 12, 8),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  'Messages',
                  style: Theme.of(context).textTheme.titleLarge?.copyWith(
                        fontWeight: FontWeight.w800,
                        letterSpacing: 0.3,
                      ),
                ),
              ),
              IconButton(
                tooltip: 'Refresh',
                icon: const Icon(Icons.refresh, size: 20),
                onPressed: _loadInitial,
              ),
            ],
          ),
        ),
        const SizedBox(height: 4),
        // Contact list
        Expanded(
          child: _contacts.isEmpty
              ? Center(
                  child: Padding(
                    padding: const EdgeInsets.all(24),
                    child: Text(
                      'No contacts yet.\nsideband contact add …',
                      textAlign: TextAlign.center,
                      style: TextStyle(
                        color: _textMuted,
                        fontSize: 13,
                        height: 1.5,
                      ),
                    ),
                  ),
                )
              : ListView.builder(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  itemCount: _contacts.length,
                  itemBuilder: (context, index) {
                    final c = _contacts[index];
                    final isSelected = _selectedContact?.name == c.name;
                    return Padding(
                      padding: const EdgeInsets.only(bottom: 2),
                      child: ListTile(
                        selected: isSelected,
                        leading: CircleAvatar(
                          radius: 18,
                          backgroundColor: c.avatarColor,
                          child: Text(
                            c.initials,
                            style: const TextStyle(
                              color: Colors.white,
                              fontSize: 13,
                              fontWeight: FontWeight.w700,
                            ),
                          ),
                        ),
                        title: Text(
                          c.name,
                          style: const TextStyle(
                            fontWeight: FontWeight.w600,
                            fontSize: 14,
                          ),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                        subtitle: Text(
                          _shortOnion(c.onion),
                          style: TextStyle(
                            fontSize: 11,
                            color: isSelected
                                ? _textMuted.withAlpha(179)
                                : _textMuted,
                          ),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                        onTap: () async {
                          setState(() => _selectedContact = c);
                          await _refreshMessages();
                        },
                      ),
                    );
                  },
                ),
        ),
        // Sidebar footer: profile + debug toggle
        Container(
          padding: const EdgeInsets.all(12),
          decoration: const BoxDecoration(
            border: Border(top: BorderSide(color: _borderColor, width: 1)),
          ),
          child: Row(
            children: [
              Icon(Icons.circle, size: 8, color: _accentTeal),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  _cli.profile,
                  style: const TextStyle(fontSize: 11, color: _textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              GestureDetector(
                onTap: () => setState(() => _showDebug = !_showDebug),
                child: Icon(
                  _showDebug ? Icons.bug_report : Icons.bug_report_outlined,
                  size: 16,
                  color: _showDebug ? _accentTeal : _textMuted,
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }

  // ── Empty state ─────────────────────────────────────────────────────────

  Widget _buildEmptyState() {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.send_rounded, size: 48, color: _textMuted.withAlpha(102)),
          const SizedBox(height: 16),
          Text(
            'Select a contact to start messaging',
            style: TextStyle(
              color: _textMuted,
              fontSize: 15,
            ),
          ),
        ],
      ),
    );
  }

  // ── Chat area ───────────────────────────────────────────────────────────

  Widget _buildChatArea() {
    final contact = _selectedContact!;

    return Column(
      children: [
        // Chat header
        _buildChatHeader(contact),
        Divider(height: 1, thickness: 1, color: _borderColor),

        // Error banner
        if (_error != null) _buildErrorBanner(),

        // Messages
        Expanded(child: _buildMessageList()),

        // Debug strip (behind toggle)
        if (_showDebug) _buildDebugStrip(),

        Divider(height: 1, thickness: 1, color: _borderColor),

        // Input area
        _buildInputArea(),
      ],
    );
  }

  Widget _buildChatHeader(Contact contact) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
      color: _surfaceDark,
      child: Row(
        children: [
          CircleAvatar(
            radius: 18,
            backgroundColor: contact.avatarColor,
            child: Text(
              contact.initials,
              style: const TextStyle(
                color: Colors.white,
                fontSize: 13,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  contact.name,
                  style: const TextStyle(
                    fontWeight: FontWeight.w700,
                    fontSize: 15,
                  ),
                ),
                const SizedBox(height: 2),
                Row(
                  children: [
                    Icon(Icons.lock, size: 10, color: _textMuted),
                    const SizedBox(width: 4),
                    Text(
                      'Encrypted via Tor',
                      style: TextStyle(fontSize: 11, color: _textMuted),
                    ),
                  ],
                ),
              ],
            ),
          ),
          // Status dot
          Tooltip(
            message: 'Connected',
            child: Container(
              width: 8,
              height: 8,
              decoration: const BoxDecoration(
                color: _accentTeal,
                shape: BoxShape.circle,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildErrorBanner() {
    return GestureDetector(
      onTap: () => setState(() => _error = null),
      child: Container(
        width: double.infinity,
        color: const Color(0xFF3A1020),
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
        child: Row(
          children: [
            const Icon(Icons.error_outline, size: 16, color: Color(0xFFFF6B6B)),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                _error!,
                style: const TextStyle(
                  color: Color(0xFFFF6B6B),
                  fontSize: 12,
                ),
              ),
            ),
            const Icon(Icons.close, size: 14, color: Color(0xFFFF6B6B)),
          ],
        ),
      ),
    );
  }

  Widget _buildMessageList() {
    if (_messages.isEmpty) {
      return Center(
        child: Text(
          'No messages yet',
          style: TextStyle(color: _textMuted, fontSize: 14),
        ),
      );
    }

    return ListView.builder(
      controller: _scrollController,
      reverse: true,
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 16),
      itemCount: _messages.length,
      itemBuilder: (context, index) {
        final m = _messages[index];
        final prevMsg =
            index < _messages.length - 1 ? _messages[index + 1] : null;
        final showDate =
            prevMsg == null || !_sameDay(m.timestamp, prevMsg.timestamp);

        return Column(
          children: [
            if (showDate) _buildDateDivider(m.timestamp),
            _buildBubble(m),
            const SizedBox(height: 2),
          ],
        );
      },
    );
  }

  Widget _buildDateDivider(DateTime dt) {
    final label = _dateLabel(dt);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: Row(
        children: [
          Expanded(child: Divider(color: _borderColor)),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Text(
              label,
              style: const TextStyle(
                fontSize: 11,
                color: _textMuted,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.3,
              ),
            ),
          ),
          Expanded(child: Divider(color: _borderColor)),
        ],
      ),
    );
  }

  Widget _buildBubble(ChatMessage m) {
    final out = m.isOutgoing;

    return Align(
      alignment: out ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.of(context).size.width * 0.7,
        ),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          margin: const EdgeInsets.only(bottom: 2),
          decoration: BoxDecoration(
            color: out ? _bubbleOut : _bubbleIn,
            borderRadius: BorderRadius.only(
              topLeft: const Radius.circular(16),
              topRight: const Radius.circular(16),
              bottomLeft: Radius.circular(out ? 16 : 4),
              bottomRight: Radius.circular(out ? 4 : 16),
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                m.text,
                style: const TextStyle(
                  fontSize: 14.5,
                  color: _textPrimary,
                  height: 1.35,
                ),
              ),
              const SizedBox(height: 4),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    _formatTime(m.timestamp),
                    style: TextStyle(
                      fontSize: 10.5,
                      color: _textMuted.withAlpha(179),
                    ),
                  ),
                  if (out) ...[
                    const SizedBox(width: 4),
                    _buildStatusIcon(m),
                  ],
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildStatusIcon(ChatMessage m) {
    if (m.status == 'sending') {
      return SizedBox(
        width: 10,
        height: 10,
        child: CircularProgressIndicator(
          strokeWidth: 1.5,
          color: _textMuted.withAlpha(153),
        ),
      );
    }
    if (m.failed) {
      return Icon(Icons.error_outline,
          size: 12, color: const Color(0xFFFF6B6B));
    }
    return Icon(Icons.done_all, size: 13, color: _accentTeal.withAlpha(204));
  }

  Widget _buildDebugStrip() {
    return Container(
      width: double.infinity,
      color: _panelDark,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
      child: Wrap(
        spacing: 12,
        runSpacing: 4,
        children: [
          _debugChip('bin', _activeBinary.split('/').last),
          _debugChip('profile', _cli.profile),
          _debugChip('last refresh',
              _lastRefreshAt == null ? 'never' : _formatTime(_lastRefreshAt!)),
          _debugChip('messages', '${_messages.length}'),
          _debugChip('sending', _sending.toString()),
        ],
      ),
    );
  }

  Widget _debugChip(String label, String value) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(
        color: _chatBgDark,
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: _borderColor),
      ),
      child: Text(
        '$label: $value',
        style: const TextStyle(fontSize: 10, color: _textMuted),
      ),
    );
  }

  Widget _buildInputArea() {
    return Container(
      color: _surfaceDark,
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 16),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Expanded(
            child: TextField(
              controller: _inputController,
              enabled: !_sending,
              minLines: 1,
              maxLines: 5,
              style: const TextStyle(fontSize: 14, color: _textPrimary),
              decoration: const InputDecoration(
                hintText: 'Send a message…',
              ),
              onSubmitted: (_) => _sending ? null : _send(),
            ),
          ),
          const SizedBox(width: 10),
          FilledButton(
            onPressed: _sending ? null : _send,
            style: FilledButton.styleFrom(
              minimumSize: const Size(44, 44),
              padding: const EdgeInsets.all(0),
            ),
            child: _sending
                ? SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: _chatBgDark.withAlpha(179),
                    ),
                  )
                : const Icon(Icons.send_rounded, size: 18),
          ),
        ],
      ),
    );
  }

  // ── Helpers ─────────────────────────────────────────────────────────────

  String _shortOnion(String onion) {
    if (onion.length <= 12) return onion;
    return '${onion.substring(0, 8)}…${onion.substring(onion.length - 6)}';
  }

  String _formatTime(DateTime dt) {
    final h = dt.hour.toString().padLeft(2, '0');
    final m = dt.minute.toString().padLeft(2, '0');
    return '$h:$m';
  }

  bool _sameDay(DateTime a, DateTime b) =>
      a.year == b.year && a.month == b.month && a.day == b.day;

  String _dateLabel(DateTime dt) {
    final now = DateTime.now();
    if (_sameDay(dt, now)) return 'Today';
    final yesterday = now.subtract(const Duration(days: 1));
    if (_sameDay(dt, yesterday)) return 'Yesterday';
    return '${dt.month}/${dt.day}/${dt.year % 100}';
  }
}
