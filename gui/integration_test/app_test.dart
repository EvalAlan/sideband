import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:sideband_gui/main.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('add-contact, group command, and group selection work end to end',
      (tester) async {
    if (Platform.isLinux) {
      expect(Platform.environment['SIDEBAND_PROFILE'], isNotEmpty);
    }

    await tester.pumpWidget(const SidebandApp(skipListener: true));
    await tester.pump(const Duration(seconds: 2));

    if (Platform.isAndroid) {
      expect(find.text('Set up Sideband'), findsOneWidget);
      await tester.enterText(
        find.widgetWithText(TextField, 'Display name'),
        'UiTest',
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Create profile'));
      await tester.pump(const Duration(seconds: 3));
    } else {
      await tester.pumpAndSettle();
    }

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

    final composer = find.byWidgetPredicate(
      (widget) => widget is TextField &&
          widget.decoration?.hintText == 'Message or /help for commands…',
      description: 'message composer',
    );
    expect(composer, findsOneWidget);

    await tester.enterText(composer, '/group-create TestGroup Rocky');
    await tester.tap(find.byIcon(Icons.send_rounded));
    await tester.pumpAndSettle();

    if (Platform.isAndroid) {
      await tester.tap(find.byIcon(Icons.arrow_back));
      await tester.pumpAndSettle();
    }

    expect(find.text('Groups'), findsOneWidget);
    expect(find.text('TestGroup'), findsWidgets);

    if (find.text('Group created').evaluate().isNotEmpty) {
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
    }
    final groupTile = find.ancestor(
      of: find.text('TestGroup').first,
      matching: find.byType(ListTile),
    );
    expect(groupTile, findsOneWidget);
    await tester.tap(groupTile);
    await tester.pumpAndSettle();
    expect(find.text('Group fan-out to 2 members'), findsOneWidget);
  });

  testWidgets('/add command routes through the active platform backend',
      (tester) async {
    await tester.pumpWidget(const SidebandApp(skipListener: true));
    await tester.pump(const Duration(seconds: 2));

    final composer = find.byWidgetPredicate(
      (widget) => widget is TextField &&
          widget.decoration?.hintText == 'Message or /help for commands…',
      description: 'message composer',
    );
    expect(composer, findsOneWidget);

    await tester.enterText(
      composer,
      '/add Adrian '
      'qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion '
      'AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA= '
      'ISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0A=',
    );
    await tester.tap(find.byIcon(Icons.send_rounded));
    await tester.pumpAndSettle();
    expect(find.text('Adrian'), findsWidgets);
  });
}
