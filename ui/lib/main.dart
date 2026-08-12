import 'package:flutter/material.dart';
import 'src/screens/dashboard_screen.dart';
import 'src/services/lifecycle_manager.dart';

import 'package:mesh_ui/src/rust/api.dart/frb_generated.dart';

void main() async {
  // Ensure flutter bindings are initialized before FFI
  WidgetsFlutterBinding.ensureInitialized();

  try {
    debugPrint('Starting RustLib.init()...');
    // Initialize FFI bindings
    await RustLib.init();
    debugPrint('Finished RustLib.init()...');

    // Initialize background lifecycle and battery/thermal manager
    LifecycleManager.instance.init();

    debugPrint('Calling runApp...');
    runApp(const MeshNodeApp());
  } catch (e, stackTrace) {
    debugPrint('Initialization failed: $e\n$stackTrace');
    runApp(MaterialApp(
      home: Scaffold(
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(16.0),
            child: SingleChildScrollView(
              child: Text(
                'Initialization Failed:\n\n$e\n\n$stackTrace',
                style: const TextStyle(color: Colors.red),
              ),
            ),
          ),
        ),
      ),
    ));
  }
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
