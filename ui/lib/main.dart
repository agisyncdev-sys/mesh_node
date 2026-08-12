import 'package:flutter/material.dart';
import 'src/screens/dashboard_screen.dart';
import 'src/services/lifecycle_manager.dart';

import 'package:mesh_ui/src/rust/api.dart/frb_generated.dart';

void main() async {
  // Ensure flutter bindings are initialized before FFI
  WidgetsFlutterBinding.ensureInitialized();

  debugPrint('Starting RustLib.init()...');
  // Initialize FFI bindings
  await RustLib.init();
  debugPrint('Finished RustLib.init()...');

  // Initialize background lifecycle and battery/thermal manager
  LifecycleManager.instance.init();

  debugPrint('Calling runApp...');
  runApp(const MeshNodeApp());
}

class MeshNodeApp extends StatelessWidget {
  const MeshNodeApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'AI Mesh Node',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        primarySwatch: Colors.blue,
        useMaterial3: true,
      ),
      home: const DashboardScreen(),
    );
  }
}
