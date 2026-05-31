import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sideband_gui/main.dart';

void main() {
  testWidgets('app boots and shows primary shell', (WidgetTester tester) async {
    await tester.pumpWidget(const SidebandApp());

    expect(find.text('Sideband'), findsOneWidget);
    expect(find.byIcon(Icons.refresh), findsOneWidget);
  });
}
