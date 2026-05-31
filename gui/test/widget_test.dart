import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sideband_gui/main.dart';

void main() {
  testWidgets('app boot does not crash', (WidgetTester tester) async {
    await tester.pumpWidget(const SidebandApp());
    await tester.pump(const Duration(milliseconds: 500));

    // App should show either loading spinner or sidebar header
    final hasLoading = find.byType(CircularProgressIndicator).evaluate().isNotEmpty;
    final hasMessages = find.text('Messages').evaluate().isNotEmpty;
    expect(hasLoading || hasMessages, isTrue);
  });
}
