import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'dart:typed_data';
import '../rust/api.dart/api.dart';
import '../services/shared_buffer.dart';
import '../services/lifecycle_manager.dart';

class DashboardScreen extends StatefulWidget {
  const DashboardScreen({super.key});

  @override
  State<DashboardScreen> createState() => _DashboardScreenState();
}

class _DashboardScreenState extends State<DashboardScreen> {
  // Config controllers
  final TextEditingController _nodeIdController =
      TextEditingController(text: 'Node-A');
  final TextEditingController _listenPortController =
      TextEditingController(text: '50061');
  final TextEditingController _nextPeerPortController =
      TextEditingController(text: '50062');
  final TextEditingController _ringSizeController =
      TextEditingController(text: '3');
  final TextEditingController _promptController = TextEditingController();

  // FFI Zero-Copy state
  SafeSharedBuffer? _sharedBuffer;
  final TextEditingController _bufferSizeController =
      TextEditingController(text: '1024');
  String _ffiStatus = 'No buffer allocated';

  // Network/Inference state
  bool _isNodeStarted = false;
  bool _isConnecting = false;
  bool _isConnectedToNextPeer = false;
  bool _isInferenceRunning = false;

  final List<Map<String, dynamic>> _messages =
      []; // type: 'user', 'inference', 'info'

  // Telemetry state
  int _ramRssMb = 0;
  int _ramVszMb = 0;
  int _latencyMs = 0;
  List<dynamic> _discoveredPeers = [];
  Timer? _telemetryTimer;
  StreamSubscription<String>? _peerStreamSub;
  StreamSubscription<Float32List>? _resultStreamSub;

  @override
  void initState() {
    super.initState();
    _startTelemetryPoll();
    LifecycleManager.instance.addListener(_onLifecycleChanged);
    _verifyModelAsset();
  }

  Future<void> _verifyModelAsset() async {
    try {
      final ByteData data =
          await rootBundle.load('assets/models/model_4bit_quantized.onnx');
      setState(() {
        _messages.add({
          'type': 'info',
          'text': 'Model Asset Verified: Loaded ${data.lengthInBytes} bytes.',
          'time': DateTime.now(),
        });
      });
    } catch (e) {
      debugPrint('Failed to load model asset: $e');
    }
  }

  void _onLifecycleChanged() {
    if (mounted) {
      setState(() {});
    }
  }

  void _startTelemetryPoll() {
    _telemetryTimer = Timer.periodic(const Duration(seconds: 2), (timer) async {
      try {
        final jsonStr = await getTelemetryJson();
        final Map<String, dynamic> data = jsonDecode(jsonStr);
        final peersJsonStr = await getDiscoveredPeersHealth();
        final List<dynamic> peersData = jsonDecode(peersJsonStr);
        if (mounted) {
          setState(() {
            _ramRssMb = data['ram_rss_mb'] ?? 0;
            _ramVszMb = data['ram_vsz_mb'] ?? 0;
            _latencyMs = data['latency_ms'] ?? 0;
            _discoveredPeers = peersData;
          });
        }
      } catch (e) {
        debugPrint('Failed to query telemetry: $e');
      }
    });
  }

  Future<void> _handleStartNode() async {
    final nodeId = _nodeIdController.text;
    final listenPort = _listenPortController.text;
    final nextPort = _nextPeerPortController.text;
    final ringSize = int.tryParse(_ringSizeController.text) ?? 3;

    if (nodeId.isEmpty || listenPort.isEmpty || nextPort.isEmpty) return;

    setState(() {
      _messages.add({
        'type': 'info',
        'text': 'Starting Ring Node [$nodeId] on port $listenPort...',
        'time': DateTime.now(),
      });
    });

    final success = await startNode(
      listenAddr: '127.0.0.1:$listenPort',
      nextPeerAddr: '127.0.0.1:$nextPort',
      ringSize: ringSize,
      nodeId: nodeId,
    );

    if (success) {
      final libp2pPort = (int.tryParse(listenPort) ?? 50000) + 1000;
      await startMeshNode(port: libp2pPort);

      _peerStreamSub?.cancel();
      _peerStreamSub = peerDiscoveryStream().listen((peerId) {
        if (mounted) {
          setState(() {
            _messages.add({
              'type': 'info',
              'text': 'libp2p: Discovered peer $peerId via mDNS',
              'time': DateTime.now(),
            });
          });
        }
      });
      
      _resultStreamSub?.cancel();
      _resultStreamSub = aggregatedResultStream().listen((tensorData) {
        if (mounted) {
          setState(() {
            _messages.add({
              'type': 'inference',
              'text': 'FINAL RING RESULT (ZK Verified): $tensorData',
              'time': DateTime.now(),
            });
          });
        }
      });
    }

    if (mounted) {
      setState(() {
        _isNodeStarted = success;
        _messages.add({
          'type': 'info',
          'text': success
              ? 'Ring Node [$nodeId] started! Next target: 127.0.0.1:$nextPort'
              : 'Failed to start Ring Node (already running).',
          'time': DateTime.now(),
        });
      });
    }
  }

  Future<void> _handleConnectToPeer() async {
    final nextPort = _nextPeerPortController.text;
    if (nextPort.isEmpty) return;

    setState(() {
      _isConnecting = true;
      _messages.add({
        'type': 'info',
        'text': 'Attempting connection to peer at port $nextPort...',
        'time': DateTime.now(),
      });
    });

    final success = await connectToPeer(peerAddr: '127.0.0.1:$nextPort');

    if (mounted) {
      setState(() {
        _isConnecting = false;
        _isConnectedToNextPeer = success;
        _messages.add({
          'type': 'info',
          'text': success
              ? 'Connected to next peer in circle!'
              : 'Connection check failed. Node will retry during All-Reduce transfers.',
          'time': DateTime.now(),
        });
      });
    }
  }

  Future<void> _handleSendPrompt() async {
    final prompt = _promptController.text;
    if (prompt.isEmpty) return;

    setState(() {
      _isInferenceRunning = true;
      _messages.add({
        'type': 'user',
        'text': prompt,
        'time': DateTime.now(),
      });
      _promptController.clear();
    });

    try {
      final originatorId = _nodeIdController.text;
      
      // Use DHT peer if available, otherwise fallback to local port config
      final targetAddr = _discoveredPeers.isNotEmpty 
          ? _discoveredPeers.first['address'] as String
          : '127.0.0.1:${_nextPeerPortController.text}';

      // Executing Rust inference and forwarding along Ring All-Reduce asynchronously
      final result = await sendPrompt(
        originatorId: originatorId,
        prompt: prompt,
        nextPeerAddr: targetAddr,
      );

      if (mounted) {
        setState(() {
          _isInferenceRunning = false;
          _messages.add({
            'type': 'inference',
            'text': result,
            'time': DateTime.now(),
          });
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _isInferenceRunning = false;
          _messages.add({
            'type': 'info',
            'text': 'Error: $e',
            'time': DateTime.now(),
          });
        });
      }
    }
  }

  @override
  void dispose() {
    _telemetryTimer?.cancel();
    _peerStreamSub?.cancel();
    _resultStreamSub?.cancel();
    _nodeIdController.dispose();
    _listenPortController.dispose();
    _nextPeerPortController.dispose();
    _ringSizeController.dispose();
    _promptController.dispose();
    _sharedBuffer?.dispose();
    _bufferSizeController.dispose();
    LifecycleManager.instance.removeListener(_onLifecycleChanged);
    super.dispose();
  }

  Future<void> _handleFFIAllocate() async {
    final size = int.tryParse(_bufferSizeController.text) ?? 1024;
    try {
      await _sharedBuffer?.dispose();
      final buf = await SafeSharedBuffer.allocate(size);
      setState(() {
        _sharedBuffer = buf;
        _ffiStatus =
            'Allocated ${buf.length} floats @ 0x${buf.address.toRadixString(16).toUpperCase()}';
        _messages.add({
          'type': 'info',
          'text':
              'FFI: Shared memory buffer allocated at 0x${buf.address.toRadixString(16).toUpperCase()} (${buf.length * 4} bytes)',
          'time': DateTime.now(),
        });
      });
    } catch (e) {
      setState(() {
        _ffiStatus = 'Allocation failed: $e';
      });
    }
  }

  Future<void> _handleFFIWrite() async {
    final buf = _sharedBuffer;
    if (buf == null) return;
    try {
      final list = buf.data;
      for (int i = 0; i < list.length; i++) {
        list[i] = i * 1.0;
      }
      setState(() {
        _ffiStatus = 'Wrote sequential f32 values in-place';
        _messages.add({
          'type': 'info',
          'text':
              'FFI: Wrote index values directly into raw buffer 0x${buf.address.toRadixString(16).toUpperCase()}',
          'time': DateTime.now(),
        });
      });
    } catch (e) {
      setState(() {
        _ffiStatus = 'Write failed: $e';
      });
    }
  }

  Future<void> _handleFFIProcess() async {
    final buf = _sharedBuffer;
    if (buf == null) return;
    try {
      final watch = Stopwatch()..start();
      await processTensorZeroCopy(
          ptr: buf.rawBuffer.ptr, len: buf.rawBuffer.len);
      watch.stop();
      setState(() {
        _ffiStatus =
            'Processed in Rust (zero-copy) in ${watch.elapsedMicroseconds} μs';
        _messages.add({
          'type': 'info',
          'text':
              'FFI: Processed in-place math in Rust core. Elapsed: ${watch.elapsedMicroseconds} μs (No allocations/copies!)',
          'time': DateTime.now(),
        });
      });
    } catch (e) {
      setState(() {
        _ffiStatus = 'Processing failed: $e';
      });
    }
  }

  void _handleFFIValidate() {
    final buf = _sharedBuffer;
    if (buf == null) return;
    try {
      final list = buf.data;
      final preview = list.take(5).join(', ');
      setState(() {
        _ffiStatus = 'Read first 5 values: [$preview]';
        _messages.add({
          'type': 'info',
          'text':
              'FFI: Checked shared buffer data in Dart -> First 5 values: [$preview]',
          'time': DateTime.now(),
        });
      });
    } catch (e) {
      setState(() {
        _ffiStatus = 'Validation failed: $e';
      });
    }
  }

  Future<void> _handleFFIFree() async {
    final buf = _sharedBuffer;
    if (buf == null) return;
    try {
      await buf.dispose();
      setState(() {
        _sharedBuffer = null;
        _ffiStatus = 'Buffer freed successfully';
        _messages.add({
          'type': 'info',
          'text':
              'FFI: Shared buffer at 0x${buf.address.toRadixString(16).toUpperCase()} explicitly freed from heap.',
          'time': DateTime.now(),
        });
      });
    } catch (e) {
      setState(() {
        _ffiStatus = 'Free failed: $e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor:
          const Color(0xFF0F0F13), // Ultra premium deep navy dark background
      appBar: AppBar(
        title: const Text(
          'AI Mesh Node Dashboard',
          style: TextStyle(fontWeight: FontWeight.bold, letterSpacing: 0.5),
        ),
        backgroundColor: const Color(0xFF16161F),
        elevation: 1,
        actions: [
          _buildStatusIndicator(),
        ],
      ),
      body: Row(
        children: [
          // Sidebar Panel: Configuration
          _buildSidebar(),

          // Main Content Panel: Chat & Gauges
          Expanded(
            child: Column(
              children: [
                // Top Telemetry Header
                _buildTelemetryHeader(),

                // Chat Messages Window
                Expanded(child: _buildChatArea()),

                // Prompt Input Box
                _buildPromptInputArea(),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildStatusIndicator() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16.0),
      child: Row(
        children: [
          Icon(
            Icons.circle,
            color: _isNodeStarted
                ? (_isConnectedToNextPeer
                    ? Colors.greenAccent
                    : Colors.orangeAccent)
                : Colors.redAccent,
            size: 12,
          ),
          const SizedBox(width: 8),
          Text(
            _isNodeStarted
                ? (_isConnectedToNextPeer ? 'Active Swarm' : 'Listening Node')
                : 'Offline',
            style: const TextStyle(fontWeight: FontWeight.w600, fontSize: 13),
          ),
        ],
      ),
    );
  }

  Widget _buildSidebar() {
    return Container(
      width: 300,
      color: const Color(0xFF16161F),
      padding: const EdgeInsets.all(16.0),
      child: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text(
              'RING CONFIGURATION',
              style: TextStyle(
                color: Colors.white60,
                fontSize: 12,
                fontWeight: FontWeight.bold,
                letterSpacing: 1.2,
              ),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: _nodeIdController,
              enabled: !_isNodeStarted,
              decoration: const InputDecoration(
                labelText: 'Node Identifier',
                border: OutlineInputBorder(),
                contentPadding:
                    EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _listenPortController,
              enabled: !_isNodeStarted,
              keyboardType: TextInputType.number,
              decoration: const InputDecoration(
                labelText: 'Listen Port (gRPC)',
                border: OutlineInputBorder(),
                contentPadding:
                    EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _nextPeerPortController,
              keyboardType: TextInputType.number,
              decoration: const InputDecoration(
                labelText: 'Next Peer Port',
                border: OutlineInputBorder(),
                contentPadding:
                    EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _ringSizeController,
              enabled: !_isNodeStarted,
              keyboardType: TextInputType.number,
              decoration: const InputDecoration(
                labelText: 'Ring Topology Size',
                border: OutlineInputBorder(),
                contentPadding:
                    EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              ),
            ),
            const SizedBox(height: 24),
            SizedBox(
              width: double.infinity,
              child: ElevatedButton.icon(
                onPressed: _isNodeStarted ? null : _handleStartNode,
                icon: const Icon(Icons.play_arrow),
                label: const Text('Start Ring Node'),
                style: ElevatedButton.styleFrom(
                  backgroundColor: const Color(0xFF3B82F6),
                  foregroundColor: Colors.white,
                  padding: const EdgeInsets.symmetric(vertical: 14),
                  shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(8)),
                ),
              ),
            ),
            const SizedBox(height: 12),
            SizedBox(
              width: double.infinity,
              child: OutlinedButton.icon(
                onPressed: (_isNodeStarted && !_isConnecting)
                    ? _handleConnectToPeer
                    : null,
                icon: _isConnecting
                    ? const SizedBox(
                        width: 16,
                        height: 16,
                        child: CircularProgressIndicator(strokeWidth: 2))
                    : const Icon(Icons.link),
                label: const Text('Check Target Connection'),
                style: OutlinedButton.styleFrom(
                  foregroundColor: Colors.white,
                  padding: const EdgeInsets.symmetric(vertical: 14),
                  side: const BorderSide(color: Colors.white24),
                  shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(8)),
                ),
              ),
            ),
            const SizedBox(height: 24),
            const Text(
              'Topology Map',
              style: TextStyle(fontWeight: FontWeight.bold, fontSize: 14),
            ),
            const SizedBox(height: 8),
            _buildTopologyStatusCard(),
            _buildFFIDemoCard(),
            _buildMobileLifecycleCard(),
          ],
        ),
      ),
    );
  }

  Widget _buildTopologyStatusCard() {
    if (!_isNodeStarted) {
      return Container(
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          color: Colors.white.withValues(alpha: 0.04),
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: Colors.white12),
        ),
        child: const Row(
          children: [
            Icon(Icons.loop, color: Colors.white24),
            SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Topology: Inactive',
                    style: TextStyle(
                        fontWeight: FontWeight.bold,
                        fontSize: 12,
                        color: Colors.white38),
                  ),
                  Text(
                    'Start node to discover peers',
                    style: TextStyle(fontSize: 11, color: Colors.white30),
                  ),
                ],
              ),
            )
          ],
        ),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Container(
          padding: const EdgeInsets.all(10),
          decoration: BoxDecoration(
            color: const Color(0xFF10B981).withValues(alpha: 0.1),
            borderRadius: BorderRadius.circular(8),
            border: Border.all(
                color: const Color(0xFF10B981).withValues(alpha: 0.2)),
          ),
          child: Row(
            children: [
              const Icon(Icons.hub, color: Color(0xFF10B981), size: 16),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  'Optimized Ring: Local -> ${_discoveredPeers.isNotEmpty ? _discoveredPeers.first['peer_id'] : _nextPeerPortController.text}',
                  style: const TextStyle(
                      fontWeight: FontWeight.bold,
                      fontSize: 11,
                      color: Color(0xFF34D399)),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
        const SizedBox(height: 8),
        ..._discoveredPeers.map((peer) {
          final isAlive = peer['is_alive'] as bool? ?? false;
          final latency = peer['latency_ms'] as int? ?? 0;
          final peerId = peer['peer_id'] as String? ?? 'Unknown';
          final address = peer['address'] as String? ?? '';

          Color latencyColor = Colors.greenAccent;
          if (latency > 150) {
            latencyColor = Colors.redAccent;
          } else if (latency > 50) {
            latencyColor = Colors.orangeAccent;
          }

          return Container(
            margin: const EdgeInsets.only(bottom: 6),
            padding: const EdgeInsets.all(8),
            decoration: BoxDecoration(
              color: Colors.white.withValues(alpha: 0.02),
              borderRadius: BorderRadius.circular(6),
              border: Border.all(color: Colors.white.withValues(alpha: 0.05)),
            ),
            child: Row(
              children: [
                Icon(
                  isAlive ? Icons.check_circle : Icons.error_outline,
                  color: isAlive ? Colors.green : Colors.red,
                  size: 14,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        peerId,
                        style: const TextStyle(
                            fontWeight: FontWeight.w600,
                            fontSize: 11,
                            color: Colors.white),
                        overflow: TextOverflow.ellipsis,
                      ),
                      Text(
                        address,
                        style: const TextStyle(
                            fontSize: 10, color: Colors.white54),
                        overflow: TextOverflow.ellipsis,
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 4),
                Text(
                  '$latency ms',
                  style: TextStyle(
                      fontWeight: FontWeight.bold,
                      fontSize: 11,
                      color: latencyColor),
                ),
              ],
            ),
          );
        }),
      ],
    );
  }

  Widget _buildFFIDemoCard() {
    return Container(
      margin: const EdgeInsets.only(top: 16),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Colors.white.withValues(alpha: 0.02),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white12),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'FFI ZERO-COPY TENSOR BRIDGE',
            style: TextStyle(
              color: Colors.white60,
              fontSize: 11,
              fontWeight: FontWeight.bold,
              letterSpacing: 1.0,
            ),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _bufferSizeController,
                  keyboardType: TextInputType.number,
                  decoration: const InputDecoration(
                    labelText: 'Buffer Size (Floats)',
                    border: OutlineInputBorder(),
                    contentPadding:
                        EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              ElevatedButton(
                onPressed: _handleFFIAllocate,
                style: ElevatedButton.styleFrom(
                  backgroundColor: const Color(0xFF3B82F6),
                  foregroundColor: Colors.white,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
                  shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(4)),
                ),
                child: const Text('Alloc', style: TextStyle(fontSize: 11)),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            'Status: $_ffiStatus',
            style: const TextStyle(
                fontSize: 10,
                color: Colors.greenAccent,
                fontFamily: 'monospace'),
          ),
          const SizedBox(height: 12),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              OutlinedButton(
                onPressed: _sharedBuffer == null ? null : _handleFFIWrite,
                style: OutlinedButton.styleFrom(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                  side: const BorderSide(color: Colors.white24),
                ),
                child: const Text('1. Write Dart',
                    style: TextStyle(fontSize: 10, color: Colors.white)),
              ),
              OutlinedButton(
                onPressed: _sharedBuffer == null ? null : _handleFFIProcess,
                style: OutlinedButton.styleFrom(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                  side: const BorderSide(color: Colors.white24),
                ),
                child: const Text('2. Math Rust',
                    style: TextStyle(fontSize: 10, color: Colors.white)),
              ),
              OutlinedButton(
                onPressed: _sharedBuffer == null ? null : _handleFFIValidate,
                style: OutlinedButton.styleFrom(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                  side: const BorderSide(color: Colors.white24),
                ),
                child: const Text('3. Read Validate',
                    style: TextStyle(fontSize: 10, color: Colors.white)),
              ),
              ElevatedButton(
                onPressed: _sharedBuffer == null ? null : _handleFFIFree,
                style: ElevatedButton.styleFrom(
                  backgroundColor: Colors.redAccent.withValues(alpha: 0.8),
                  foregroundColor: Colors.white,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                ),
                child:
                    const Text('Free Buffer', style: TextStyle(fontSize: 10)),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildMobileLifecycleCard() {
    final battery = LifecycleManager.instance.batteryLevel;
    final thermal = LifecycleManager.instance.thermalState;
    final isThrottled = LifecycleManager.instance.isThrottled;
    final fgActive = LifecycleManager.instance.isAndroidForegroundServiceActive;
    final bgActive = LifecycleManager.instance.isIOSBackgroundTaskRegistered;

    IconData batteryIcon = Icons.battery_full;
    Color batteryColor = Colors.greenAccent;
    if (battery < 20) {
      batteryIcon = Icons.battery_alert;
      batteryColor = Colors.redAccent;
    } else if (battery < 50) {
      batteryIcon = Icons.battery_charging_full;
      batteryColor = Colors.orangeAccent;
    }

    Color thermalColor = Colors.greenAccent;
    if (thermal == ThermalState.serious || thermal == ThermalState.critical) {
      thermalColor = Colors.redAccent;
    } else if (thermal == ThermalState.fair) {
      thermalColor = Colors.orangeAccent;
    }

    return Container(
      margin: const EdgeInsets.only(top: 16),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Colors.white.withValues(alpha: 0.02),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white12),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'MOBILE OS LIFECYCLE & TELEMETRY',
            style: TextStyle(
              color: Colors.white60,
              fontSize: 11,
              fontWeight: FontWeight.bold,
              letterSpacing: 1.0,
            ),
          ),
          const SizedBox(height: 12),
          // Sensor Metrics
          Wrap(
            spacing: 12,
            runSpacing: 6,
            children: [
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(batteryIcon, color: batteryColor, size: 16),
                  const SizedBox(width: 6),
                  Text('Battery: $battery%',
                      style: const TextStyle(fontSize: 11)),
                ],
              ),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(Icons.thermostat, color: thermalColor, size: 16),
                  const SizedBox(width: 6),
                  Text('Temp: ${thermal.name.toUpperCase()}',
                      style: const TextStyle(fontSize: 11)),
                ],
              ),
            ],
          ),
          const SizedBox(height: 10),
          // Platform services status
          Wrap(
            spacing: 12,
            runSpacing: 4,
            children: [
              Text(
                'Android Foreground: ${fgActive ? "ACTIVE" : "INACTIVE"}',
                style: TextStyle(
                    fontSize: 9,
                    color: fgActive ? Colors.greenAccent : Colors.white24),
              ),
              Text(
                'iOS BG Tasks: ${bgActive ? "READY" : "INACTIVE"}',
                style: TextStyle(
                    fontSize: 9,
                    color: bgActive ? Colors.greenAccent : Colors.white24),
              ),
            ],
          ),
          if (isThrottled) ...[
            const SizedBox(height: 10),
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
              decoration: BoxDecoration(
                color: Colors.redAccent.withValues(alpha: 0.1),
                border:
                    Border.all(color: Colors.redAccent.withValues(alpha: 0.3)),
                borderRadius: BorderRadius.circular(4),
              ),
              child: const Row(
                children: [
                  Icon(Icons.warning_amber_rounded,
                      color: Colors.redAccent, size: 14),
                  SizedBox(width: 6),
                  Expanded(
                    child: Text(
                      'THROTTLING: work responsibility reduced in Rust',
                      style: TextStyle(
                          fontSize: 9,
                          color: Colors.redAccent,
                          fontWeight: FontWeight.bold),
                    ),
                  ),
                ],
              ),
            ),
          ],
          const SizedBox(height: 12),
          const Text(
            'OS TELEMETRY SIMULATORS',
            style: TextStyle(
                color: Colors.white38,
                fontSize: 9,
                fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 6),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              OutlinedButton(
                onPressed: () => LifecycleManager.instance.setBatteryLevel(15),
                style: OutlinedButton.styleFrom(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                  side: const BorderSide(color: Colors.white24),
                ),
                child: const Text('Low Battery (15%)',
                    style: TextStyle(fontSize: 9, color: Colors.white)),
              ),
              OutlinedButton(
                onPressed: () => LifecycleManager.instance
                    .setThermalState(ThermalState.serious),
                style: OutlinedButton.styleFrom(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                  side: const BorderSide(color: Colors.white24),
                ),
                child: const Text('Overheating',
                    style: TextStyle(fontSize: 9, color: Colors.white)),
              ),
              ElevatedButton(
                onPressed: () {
                  LifecycleManager.instance.setBatteryLevel(98);
                  LifecycleManager.instance
                      .setThermalState(ThermalState.normal);
                },
                style: ElevatedButton.styleFrom(
                  backgroundColor: const Color(0xFF10B981),
                  foregroundColor: Colors.white,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                ),
                child: const Text('Restore Normals',
                    style: TextStyle(fontSize: 9)),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildTelemetryHeader() {
    return Container(
      color: const Color(0xFF16161F),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          mainAxisAlignment: MainAxisAlignment.spaceAround,
          children: [
            _buildTelemetryGauge('Memory Footprint (RSS)', '$_ramRssMb MB',
                Icons.memory, Colors.cyanAccent),
            const SizedBox(width: 24),
            _buildTelemetryGauge('Virtual Memory (VSZ)', '$_ramVszMb MB',
                Icons.pie_chart_outline, Colors.purpleAccent),
            const SizedBox(width: 24),
            _buildTelemetryGauge('Hop Latency', '$_latencyMs ms', Icons.speed,
                Colors.greenAccent),
          ],
        ),
      ),
    );
  }

  Widget _buildTelemetryGauge(
      String label, String value, IconData icon, Color color) {
    return Row(
      children: [
        Icon(icon, color: color, size: 24),
        const SizedBox(width: 12),
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(label,
                style: const TextStyle(color: Colors.white54, fontSize: 11)),
            Text(value,
                style: const TextStyle(
                    color: Colors.white,
                    fontWeight: FontWeight.bold,
                    fontSize: 15)),
          ],
        )
      ],
    );
  }

  Widget _buildChatArea() {
    if (_messages.isEmpty) {
      return Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(Icons.chat_bubble_outline,
                size: 48, color: Colors.white.withValues(alpha: 0.2)),
            const SizedBox(height: 12),
            Text(
              'No communications logged yet.\nSetup node and send a prompt to start local AI & P2P ring pass.',
              textAlign: TextAlign.center,
              style: TextStyle(
                  color: Colors.white.withValues(alpha: 0.4), fontSize: 13),
            )
          ],
        ),
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.all(16),
      itemCount: _messages.length,
      itemBuilder: (context, index) {
        final msg = _messages[index];
        final type = msg['type'];
        final text = msg['text'];

        if (type == 'info') {
          return Center(
            child: Container(
              margin: const EdgeInsets.symmetric(vertical: 8),
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
              decoration: BoxDecoration(
                color: Colors.blue.withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(16),
                border: Border.all(color: Colors.blue.withValues(alpha: 0.2)),
              ),
              child: Text(
                text,
                style: const TextStyle(
                    color: Color(0xFF90CDF4),
                    fontSize: 11,
                    fontWeight: FontWeight.w500),
              ),
            ),
          );
        }

        final isUser = type == 'user';
        return Align(
          alignment: isUser ? Alignment.centerRight : Alignment.centerLeft,
          child: Container(
            margin: const EdgeInsets.symmetric(vertical: 6),
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
            constraints: const BoxConstraints(maxWidth: 600),
            decoration: BoxDecoration(
              color: isUser
                  ? const Color(0xFF3B82F6)
                  : Colors.white.withValues(alpha: 0.06),
              borderRadius: BorderRadius.only(
                topLeft: const Radius.circular(12),
                topRight: const Radius.circular(12),
                bottomLeft: Radius.circular(isUser ? 12 : 0),
                bottomRight: Radius.circular(isUser ? 0 : 12),
              ),
              border: isUser ? null : Border.all(color: Colors.white12),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  isUser ? 'Local Prompt' : 'AI Output (Via Rust ONNX & Ring)',
                  style: TextStyle(
                    color: isUser ? Colors.white70 : const Color(0xFF10B981),
                    fontWeight: FontWeight.bold,
                    fontSize: 10,
                    letterSpacing: 0.5,
                  ),
                ),
                const SizedBox(height: 4),
                Text(
                  text,
                  style: const TextStyle(
                      color: Colors.white, fontSize: 13, height: 1.4),
                ),
              ],
            ),
          ),
        );
      },
    );
  }

  Widget _buildPromptInputArea() {
    return Container(
      padding: const EdgeInsets.all(16),
      color: const Color(0xFF16161F),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _promptController,
              enabled: _isNodeStarted && !_isInferenceRunning,
              decoration: InputDecoration(
                hintText: _isNodeStarted
                    ? 'Enter prompt to process via ONNX & ring-pass...'
                    : 'Please start the node first to type prompts',
                border: const OutlineInputBorder(),
                contentPadding:
                    const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
              ),
              onSubmitted: (_) => _handleSendPrompt(),
            ),
          ),
          const SizedBox(width: 12),
          ElevatedButton(
            onPressed: (_isNodeStarted && !_isInferenceRunning)
                ? _handleSendPrompt
                : null,
            style: ElevatedButton.styleFrom(
              backgroundColor: const Color(0xFF10B981),
              foregroundColor: Colors.white,
              padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 16),
              shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(8)),
            ),
            child: _isInferenceRunning
                ? const SizedBox(
                    width: 20,
                    height: 20,
                    child: CircularProgressIndicator(
                        strokeWidth: 2, color: Colors.white))
                : const Icon(Icons.send),
          )
        ],
      ),
    );
  }
}
