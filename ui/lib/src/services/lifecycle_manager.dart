import 'package:flutter/material.dart';
import '../rust/api.dart/api.dart' as rust_api;

enum ThermalState {
  normal,
  fair,
  serious,
  critical,
}

/// Manages native mobile operating system lifecycles, background execution rules,
/// and automated battery/thermal throttling logic.
class LifecycleManager with WidgetsBindingObserver {
  static final LifecycleManager instance = LifecycleManager._internal();

  // Lifecycle states
  bool _isThrottled = false;
  int _batteryLevel = 100; // Mock battery percentage
  ThermalState _thermalState = ThermalState.normal;
  bool _isAndroidForegroundServiceActive = false;
  bool _isIOSBackgroundTaskRegistered = false;

  // Change listeners
  final List<VoidCallback> _listeners = [];

  LifecycleManager._internal();

  /// Starts lifecycle and sensor monitoring.
  void init() {
    WidgetsBinding.instance.addObserver(this);

    // In a real mobile environment, we would listen to native platform channels:
    // - BatteryManager (Android) & UIDevice (iOS)
    // - PowerManager (Android) & NSProcessInfo (iOS) for thermals
    _simulateBatteryAndThermalProfiling();
  }

  void addListener(VoidCallback listener) {
    _listeners.add(listener);
  }

  void removeListener(VoidCallback listener) {
    _listeners.remove(listener);
  }

  void _notifyListeners() {
    for (final listener in _listeners) {
      listener();
    }
  }

  // Telemetry getters
  int get batteryLevel => _batteryLevel;
  ThermalState get thermalState => _thermalState;
  bool get isThrottled => _isThrottled;
  bool get isAndroidForegroundServiceActive =>
      _isAndroidForegroundServiceActive;
  bool get isIOSBackgroundTaskRegistered => _isIOSBackgroundTaskRegistered;

  /// Simulates battery drain and thermal shifts to verify automatic throttling.
  void _simulateBatteryAndThermalProfiling() {
    // Start Android foreground service notification mockup
    _isAndroidForegroundServiceActive = true;
    // Register iOS background tasks mockup
    _isIOSBackgroundTaskRegistered = true;
    _notifyListeners();
  }

  /// Manually update battery level (for testing/simulation).
  void setBatteryLevel(int level) {
    _batteryLevel = level;
    _checkThrottlingStatus();
  }

  /// Manually update thermal state (for testing/simulation).
  void setThermalState(ThermalState state) {
    _thermalState = state;
    _checkThrottlingStatus();
  }

  /// Evaluates battery and thermal thresholds and triggers Rust FFI resource updates.
  void _checkThrottlingStatus() {
    final shouldThrottle = _batteryLevel < 20 ||
        _thermalState == ThermalState.serious ||
        _thermalState == ThermalState.critical;

    if (shouldThrottle != _isThrottled) {
      _isThrottled = shouldThrottle;
      rust_api.updateNodeThrottling(lowPowerMode: shouldThrottle);
      _notifyListeners();
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    debugPrint('[Lifecycle] Mobile OS lifecycle state changed to: $state');

    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.detached) {
      // Graceful leave hook triggers instantly to preserve Ring integrity before suspension
      debugPrint(
          '[Lifecycle] DETACHED/PAUSED state detected. Triggering graceful network leave hook.');
      rust_api.notifyGracefulLeave();
    }
  }

  /// Cleans up observers.
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
  }
}
