import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:sideband_gui/main.dart';
import 'package:zxing2/qrcode.dart';

void main() {
  test('decodeQrImage reads a Sideband contact QR image', () {
    const command =
        '/add Rocky example.onion ed25519-public-key x25519-public-key';
    final matrix = Encoder.encode(command, ErrorCorrectionLevel.m).matrix!;
    const scale = 4;
    const quietZone = 4;
    final image = img.Image(
      width: (matrix.width + quietZone * 2) * scale,
      height: (matrix.height + quietZone * 2) * scale,
    );
    img.fill(image, color: img.ColorRgb8(255, 255, 255));
    for (var y = 0; y < matrix.height; y++) {
      for (var x = 0; x < matrix.width; x++) {
        if (matrix.get(x, y) != 1) continue;
        img.fillRect(
          image,
          x1: (x + quietZone) * scale,
          y1: (y + quietZone) * scale,
          x2: (x + quietZone + 1) * scale - 1,
          y2: (y + quietZone + 1) * scale - 1,
          color: img.ColorRgb8(0, 0, 0),
        );
      }
    }

    expect(decodeQrImage(img.encodePng(image)), command);
  });

  test('parseAddCommandContact preserves base64 +/= characters', () {
    // A real /share line whose keys contain '+', '/', and trailing '='.
    const line =
        '/add Rocky qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion '
        'fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w= '
        'K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=';
    final c = parseAddCommandContact(line);
    expect(c, isNotNull);
    expect(c!.name, 'Rocky');
    expect(c.pubkey, 'fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w=');
    expect(c.x25519Pubkey, 'K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=');
  });

  test('parseAddCommandContact recovers keys concatenated by a lost space', () {
    // The space between the two 44-char keys is gone (wrapped-copy artifact).
    const line =
        '/add Rocky qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion '
        'UyLPBmDO9XHwOrbYei96QQjIozjEhxa42OuCPxmS+m0=djg7VKfYC9n8onjPu4b7opLPRDjKA4URhyTf7Xd3hCw=';
    final c = parseAddCommandContact(line);
    expect(c, isNotNull);
    expect(c!.pubkey, 'UyLPBmDO9XHwOrbYei96QQjIozjEhxa42OuCPxmS+m0=');
    expect(c.x25519Pubkey, 'djg7VKfYC9n8onjPu4b7opLPRDjKA4URhyTf7Xd3hCw=');
  });

  test('contact security label shows ratchet as strongest state', () {
    const contact = Contact(
      name: 'bob',
      onion: 'example.onion',
      pubkey: 'ed25519',
      x25519Pubkey: 'x25519',
      ratchetActive: true,
    );

    expect(contact.securityIcon, '🔒');
    expect(contact.securityLabel, 'Double Ratchet');
    expect(contact.securityDescription, contains('forward secrecy'));
  });

  test('contact security label falls back to static key state', () {
    const contact = Contact(
      name: 'bob',
      onion: 'example.onion',
      pubkey: 'ed25519',
      x25519Pubkey: 'x25519',
      ratchetActive: false,
    );

    expect(contact.securityIcon, '🔐');
    expect(contact.securityLabel, 'Static key');
  });

  test('group label shows title and participant count including self', () {
    const group = GroupInfo(
      id: 'g1',
      title: 'Ops',
      members: ['alice', 'bob'],
    );

    expect(group.sidebarLabel, 'Ops');
    expect(group.participantCount, 3);
    expect(group.memberSummary, '3 members');
  });

  test('known empty contact history does not fall back to global history', () {
    expect(
        shouldFallbackToGlobalHistory(
          groupSelected: false,
          filteredHistoryEmpty: true,
          contact: 'alice',
          knownContacts: const ['alice', 'bob'],
        ),
        isFalse);
  });

  test('group create args include title and repeated members', () {
    expect(
      groupCreateArgs(
          profile: '/tmp/p', title: 'Ops', members: const ['alice', 'bob']),
      [
        'group',
        'create',
        '--profile',
        '/tmp/p',
        '--title',
        'Ops',
        '--member',
        'alice',
        '--member',
        'bob',
        '--json'
      ],
    );
  });

  test('group management args use backend group commands', () {
    expect(
      groupDeleteArgs(profile: '/tmp/p', group: 'Ops'),
      ['group', 'delete', '--profile', '/tmp/p', '--group', 'Ops'],
    );
    expect(
      groupRenameArgs(profile: '/tmp/p', group: 'g1', title: 'Homies'),
      [
        'group',
        'rename',
        '--profile',
        '/tmp/p',
        '--group',
        'g1',
        '--title',
        'Homies',
        '--json'
      ],
    );
    expect(
      groupMemberMutationArgs(
          profile: '/tmp/p', action: 'member-add', group: 'g1', member: 'bob'),
      [
        'group',
        'member-add',
        '--profile',
        '/tmp/p',
        '--group',
        'g1',
        '--member',
        'bob',
        '--json'
      ],
    );
  });

  test('raw group payload messages normalize to group chat bodies', () {
    const raw =
        '{"kind":"group_message","group_id":"g1","group_title":"SecX","members":["Alan","Zimbro"],"body":"Ping"}';
    const msg = ChatMsg(
      id: 1,
      direction: 'in',
      status: 'delivered',
      contact: 'Zimbro',
      group: '',
      text: raw,
      tsMs: 100,
    );

    final normalized = normalizeRawGroupPayloadMessage(msg);
    expect(normalized.group, 'g1');
    expect(normalized.text, 'Ping');
    expect(normalized.contact, 'Zimbro');
  });

  test('file attachment parser recognizes received and sent images', () {
    final received = parseAttachmentText('[file received: /tmp/cat photo.png]');
    expect(received, isNotNull);
    expect(received!.label, 'cat photo.png');
    expect(received.path, '/tmp/cat photo.png');
    expect(received.image, isTrue);

    final sent = parseAttachmentText(
        '[file sent: /home/alan/pic.webp (123 bytes, inline)]');
    expect(sent, isNotNull);
    expect(sent!.label, 'pic.webp');
    expect(sent.path, '/home/alan/pic.webp');
    expect(sent.image, isTrue);

    final doc =
        parseAttachmentText('[file sent: notes.txt (123 bytes, 2 chunks)]');
    expect(doc, isNotNull);
    expect(doc!.label, 'notes.txt');
    expect(doc.image, isFalse);

    final failed = parseAttachmentText('[file received failed: cat.png]');
    expect(failed, isNotNull);
    expect(failed!.path, isEmpty);
    expect(failed.label, '[file received failed: cat.png]');
    expect(failed.image, isFalse);
  });

  test('contact views hide raw group payload rows and groups recover them', () {
    const raw =
        '{"kind":"group_message","group_id":"g1","group_title":"SecX","members":["Alan","Zimbro"],"body":"Ho"}';
    const badDm = ChatMsg(
      id: 2,
      direction: 'in',
      status: 'failed',
      contact: 'Zimbro',
      group: '',
      text: raw,
      tsMs: 200,
    );
    const realDm = ChatMsg(
      id: 3,
      direction: 'in',
      status: 'delivered',
      contact: 'Zimbro',
      group: '',
      text: 'actual dm',
      tsMs: 300,
    );

    final normalized =
        [badDm, realDm].map(normalizeRawGroupPayloadMessage).toList();
    expect(
        visibleContactMessages(normalized).map((m) => m.text), ['actual dm']);

    final recovered = mergeRecoveredGroupMessages(
      groupRows: const [],
      globalRows: normalized,
      groupId: 'g1',
      limit: 80,
    );
    expect(recovered, hasLength(1));
    expect(recovered.first.text, 'Ho');
    expect(recovered.first.group, 'g1');
  });

  testWidgets('app boot does not crash', (WidgetTester tester) async {
    await tester.pumpWidget(const SidebandApp());
    await tester.pump(const Duration(milliseconds: 500));
    final hasLoading =
        find.byType(CircularProgressIndicator).evaluate().isNotEmpty;
    final hasMessages = find.text('Messages').evaluate().isNotEmpty;
    expect(hasLoading || hasMessages, isTrue);
  });

  group('transfer string parsing', () {
    test('parses hash from outbound line', () {
      expect(
        parseTransferHash(
            'outbound abc123 -> bob chunk 3/10 file=photo.jpg'),
        'abc123',
      );
    });

    test('parses key from incoming line', () {
      expect(parseTransferHash('incoming deadbeef chunks 2/5'), 'deadbeef');
    });

    test('returns null for unrecognized line', () {
      expect(parseTransferHash('garbage'), isNull);
      expect(parseTransferHash(''), isNull);
    });

    test('classifies outbound vs incoming', () {
      expect(
          isOutboundTransfer('outbound abc123 -> bob chunk 1/2 file=x'), isTrue);
      expect(isOutboundTransfer('incoming deadbeef chunks 1/2'), isFalse);
    });
  });

  group('notification helpers', () {
    test('short body is passed through untouched', () {
      expect(notificationBody('hello'), 'hello');
    });

    test('long body is truncated with an ellipsis', () {
      final long = 'x' * 200;
      final body = notificationBody(long);
      expect(body.length, 81); // 80 chars + ellipsis
      expect(body.endsWith('…'), isTrue);
    });

    test('respects a custom max length', () {
      expect(notificationBody('abcdef', maxLen: 3), 'abc…');
    });

    test('trims whitespace before measuring', () {
      expect(notificationBody('  hi  '), 'hi');
    });

    test('per-contact id is stable and non-negative', () {
      final a = notificationIdForContact('bob');
      final b = notificationIdForContact('bob');
      expect(a, b);
      expect(a, greaterThanOrEqualTo(0));
      expect(notificationIdForContact('alice'),
          isNot(notificationIdForContact('bob')));
    });
  });

  group('attachment path validation', () {
    // The MethodChannel profilePath already ends in .sideband, so the
    // downloads dir is <profile>/downloads.
    const profile = '/data/user/0/com.evalalan.sideband/files/.sideband';

    test('accepts files directly under downloads', () {
      expect(
        isUnderDownloadsDir('$profile/downloads/photo.jpg', profile),
        isTrue,
      );
    });

    test('accepts nested files under downloads', () {
      expect(
        isUnderDownloadsDir('$profile/downloads/sub/photo.jpg', profile),
        isTrue,
      );
    });

    test('rejects paths outside downloads', () {
      expect(
        isUnderDownloadsDir('$profile/identity.toml', profile),
        isFalse,
      );
      expect(
        isUnderDownloadsDir('/storage/emulated/0/Pictures/x.jpg', profile),
        isFalse,
      );
    });

    test('rejects traversal escapes', () {
      expect(
        isUnderDownloadsDir(
            '$profile/downloads/../../identity.toml', profile),
        isFalse,
      );
    });

    test('rejects empty inputs', () {
      expect(isUnderDownloadsDir('', profile), isFalse);
      expect(isUnderDownloadsDir('$profile/downloads/x', ''), isFalse);
    });
  });
}
