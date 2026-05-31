import 'package:flutter_test/flutter_test.dart';
import 'package:sideband_gui/main.dart';

void main() {
  testWidgets('app boots and shows WIP title', (WidgetTester tester) async {
    await tester.pumpWidget(const SidebandApp());

    expect(find.text('Sideband GUI (WIP)'), findsOneWidget);
    expect(find.text('Backend bridge not wired yet.'), findsOneWidget);
  });
}
