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
      home: const HomeScreen(),
      debugShowCheckedModeBanner: false,
    );
  }
}

class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Sideband GUI (WIP)')),
      body: const Padding(
        padding: EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Backend bridge not wired yet.'),
            SizedBox(height: 8),
            Text('Next: flutter_rust_bridge bindings to src/app_api.rs'),
          ],
        ),
      ),
    );
  }
}
