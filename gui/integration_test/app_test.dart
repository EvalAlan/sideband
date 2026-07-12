import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:sideband_gui/main.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('desktop add-contact flow persists through the real CLI backend',
      (tester) async {
    if (!Platform.isLinux) return;
    expect(Platform.environment['SIDEBAND_PROFILE'], isNotEmpty);

    await tester.pumpWidget(const SidebandApp(skipListener: true));
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Add contact'));
    await tester.pumpAndSettle();

    const values = <String, String>{
      'Name': 'Rocky',
      'Onion address':
          'qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion',
      'Ed25519 pubkey': 'fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w=',
      'X25519 pubkey': 'K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk=',
    };
    for (final entry in values.entries) {
      await tester.enterText(
        find.widgetWithText(TextField, entry.key),
        entry.value,
      );
    }
    await tester.tap(find.widgetWithText(FilledButton, 'Add'));
    await tester.pumpAndSettle();

    expect(find.text('Rocky'), findsWidgets);
  });
}
