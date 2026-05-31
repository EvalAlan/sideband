import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

void main() {
  runApp(const SidebandApp());
}

class SidebandApp extends StatelessWidget {
  const SidebandApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Sideband',
      theme: ThemeData.dark(useMaterial3: true),
      home: const ChatScreen(),
      debugShowCheckedModeBanner: false,
    );
  }
}

class Contact {
  Contact({required this.name, required this.onion});

  final String name;
  final String onion;
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
}

class SidebandCli {
  SidebandCli();

  final String? _overridePath = Platform.environment['SIDEBAND_BIN'];

  Future<String> _run(List<String> args) async {
    final executable = await _resolveExecutable();
    final result = await Process.run(executable, args);
    if (result.exitCode != 0) {
      throw Exception((result.stderr as String).trim().isEmpty
          ? 'sideband failed with code ${result.exitCode}'
          : (result.stderr as String).trim());
    }
    return (result.stdout as String).trim();
  }

  Future<String> _resolveExecutable() async {
    final overridePath = _overridePath;
    if (overridePath != null && overridePath.isNotEmpty) {
      return overridePath;
    }

    // Running from gui/, binary from cargo build is ../target/debug/sideband.
    final local = File('../target/debug/sideband');
    if (await local.exists()) {
      return local.path;
    }

    // Fallback to PATH.
    return 'sideband';
  }

  Future<List<Contact>> contacts() async {
    final raw = await _run(['contact', 'list']);
    if (raw.isEmpty) {
      return [];
    }

    return raw.split('\n').where((line) => line.trim().isNotEmpty).map((line) {
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

  Future<List<ChatMessage>> history({String? contact, int limit = 80}) async {
    final args = ['history', '--limit', '$limit'];
    if (contact != null && contact.trim().isNotEmpty) {
      args.addAll(['--contact', contact.trim()]);
    }

    final raw = await _run(args);
    if (raw.isEmpty) {
      return [];
    }

    final pattern = RegExp(
      r'^\[(\d+)\]\s+(in|out)\s+(\S+)\s+(.+?)\s{2,}(.+)\s+ts=(\d+)$',
    );

    final parsed = <ChatMessage>[];
    for (final line
        in raw.split('\n').where((line) => line.trim().isNotEmpty)) {
      final m = pattern.firstMatch(line.trim());
      if (m == null) {
        // Don't drop the entire timeline because one row format changed.
        continue;
      }
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

    return parsed.reversed.toList();
  }

  Future<void> sendMessage(
      {required String to, required String message}) async {
    await _run(['send', '--to', to, '--message', message]);
  }
}

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _cli = SidebandCli();
  final _inputController = TextEditingController();

  List<Contact> _contacts = [];
  List<ChatMessage> _messages = [];
  Contact? _selectedContact;
  bool _loading = true;
  bool _sending = false;
  String? _error;
  Timer? _refreshTimer;

  @override
  void initState() {
    super.initState();
    unawaited(_loadInitial());
    _refreshTimer = Timer.periodic(const Duration(seconds: 4), (_) {
      unawaited(_refreshMessages(silent: true));
    });
  }

  @override
  void dispose() {
    _refreshTimer?.cancel();
    _inputController.dispose();
    super.dispose();
  }

  Future<void> _loadInitial() async {
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      final contacts = await _cli.contacts();
      Contact? selected = _selectedContact;
      if (selected == null && contacts.isNotEmpty) {
        selected = contacts.first;
      } else if (selected != null) {
        final idx = contacts.indexWhere((c) => c.name == selected!.name);
        selected = idx >= 0
            ? contacts[idx]
            : (contacts.isNotEmpty ? contacts.first : null);
      }

      final history = await _cli.history(contact: selected?.name);

      setState(() {
        _contacts = contacts;
        _selectedContact = selected;
        _messages = history;
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
    if (_selectedContact == null) {
      return;
    }
    try {
      final history = await _cli.history(contact: _selectedContact!.name);
      setState(() {
        _messages = history;
        if (!silent) {
          _error = null;
        }
      });
    } catch (e) {
      if (!silent) {
        setState(() => _error = e.toString());
      }
    }
  }

  Future<void> _send() async {
    final contact = _selectedContact;
    if (contact == null) {
      return;
    }
    final text = _inputController.text.trim();
    if (text.isEmpty) {
      return;
    }

    setState(() {
      _sending = true;
      _error = null;
    });

    try {
      await _cli.sendMessage(to: contact.name, message: text);
      _inputController.clear();
      await _refreshMessages();
    } catch (e) {
      setState(() => _error = e.toString());
    } finally {
      if (mounted) {
        setState(() => _sending = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Sideband'),
        actions: [
          IconButton(
            tooltip: 'Reload',
            onPressed: _loading ? null : _loadInitial,
            icon: const Icon(Icons.refresh),
          ),
        ],
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : Row(
              children: [
                SizedBox(
                  width: 300,
                  child: _buildContactPane(),
                ),
                const VerticalDivider(width: 1),
                Expanded(child: _buildChatPane()),
              ],
            ),
    );
  }

  Widget _buildContactPane() {
    return Column(
      children: [
        ListTile(
          title: const Text('Contacts'),
          subtitle: Text('${_contacts.length} total'),
          trailing: IconButton(
            tooltip: 'Refresh contacts',
            onPressed: _loadInitial,
            icon: const Icon(Icons.sync),
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: _contacts.isEmpty
              ? const Center(
                  child: Text(
                      'No contacts. Add via CLI: sideband contact add ...'))
              : ListView.builder(
                  itemCount: _contacts.length,
                  itemBuilder: (context, index) {
                    final contact = _contacts[index];
                    final selected = _selectedContact?.name == contact.name;
                    return ListTile(
                      selected: selected,
                      title: Text(contact.name),
                      subtitle: Text(
                        contact.onion,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                      onTap: () async {
                        setState(() => _selectedContact = contact);
                        await _refreshMessages();
                      },
                    );
                  },
                ),
        ),
      ],
    );
  }

  Widget _buildChatPane() {
    final selected = _selectedContact;
    if (selected == null) {
      return const Center(child: Text('No contact selected.'));
    }

    return Column(
      children: [
        ListTile(
          title: Text(selected.name),
          subtitle: Text('${selected.onion}\nprofile: ~/.sideband'),
          isThreeLine: true,
        ),
        const Divider(height: 1),
        if (_error != null)
          Container(
            width: double.infinity,
            color: Colors.red.shade900,
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Text(
              _error!,
              style: const TextStyle(color: Colors.white),
            ),
          ),
        Expanded(
          child: _messages.isEmpty
              ? const Center(child: Text('No messages yet.'))
              : ListView.builder(
                  reverse: true,
                  itemCount: _messages.length,
                  itemBuilder: (context, index) {
                    final m = _messages[index];
                    return Align(
                      alignment: m.isOutgoing
                          ? Alignment.centerRight
                          : Alignment.centerLeft,
                      child: ConstrainedBox(
                        constraints: const BoxConstraints(maxWidth: 680),
                        child: Card(
                          color: m.isOutgoing
                              ? Colors.blueGrey.shade900
                              : Colors.grey.shade900,
                          child: Padding(
                            padding: const EdgeInsets.all(10),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  m.text,
                                  style: const TextStyle(fontSize: 15),
                                ),
                                const SizedBox(height: 6),
                                Text(
                                  '${_formatTime(m.timestamp)} • ${m.status} • #${m.id}',
                                  style: TextStyle(
                                    fontSize: 11,
                                    color: Colors.grey.shade400,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    );
                  },
                ),
        ),
        const Divider(height: 1),
        Padding(
          padding: const EdgeInsets.all(12),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _inputController,
                  enabled: !_sending,
                  minLines: 1,
                  maxLines: 4,
                  decoration: const InputDecoration(
                    hintText: 'Type a message…',
                    border: OutlineInputBorder(),
                  ),
                  onSubmitted: (_) => _sending ? null : _send(),
                ),
              ),
              const SizedBox(width: 10),
              FilledButton.icon(
                onPressed: _sending ? null : _send,
                icon: _sending
                    ? const SizedBox(
                        width: 14,
                        height: 14,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.send),
                label: const Text('Send'),
              ),
            ],
          ),
        ),
      ],
    );
  }

  String _formatTime(DateTime dt) {
    final h = dt.hour.toString().padLeft(2, '0');
    final m = dt.minute.toString().padLeft(2, '0');
    final s = dt.second.toString().padLeft(2, '0');
    return '$h:$m:$s';
  }
}
