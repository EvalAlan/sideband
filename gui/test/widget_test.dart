import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sideband_gui/main.dart';

void main() {
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

  testWidgets('app boot does not crash', (WidgetTester tester) async {
    await tester.pumpWidget(const SidebandApp());
    await tester.pump(const Duration(milliseconds: 500));
    final hasLoading =
        find.byType(CircularProgressIndicator).evaluate().isNotEmpty;
    final hasMessages = find.text('Messages').evaluate().isNotEmpty;
    expect(hasLoading || hasMessages, isTrue);
  });
}
