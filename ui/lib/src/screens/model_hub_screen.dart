import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:dio/dio.dart';
import 'package:path_provider/path_provider.dart';
import '../rust/api.dart/api.dart';

class ModelHubScreen extends StatefulWidget {
  const ModelHubScreen({super.key});

  @override
  State<ModelHubScreen> createState() => _ModelHubScreenState();
}

class _ModelHubScreenState extends State<ModelHubScreen> {
  final Dio _dio = Dio();
  
  // Dummy list of open source models for MVP
  final List<Map<String, dynamic>> _models = [
    {
      'name': 'Identity Pipeline (Built-in)',
      'description': 'Dummy fast model for testing pipeline parallelism.',
      'size': '4 KB',
      'url': 'default',
      'isDownloaded': true,
      'isDownloading': false,
      'progress': 0.0,
      'localPath': 'default',
    },
    {
      'name': 'TinyLlama 1.1B (Slice 1)',
      'description': 'First chunk of TinyLlama for edge devices.',
      'size': '2.2 GB',
      'url': 'https://huggingface.co/dummy/tiny-llama.onnx',
      'isDownloaded': false,
      'isDownloading': false,
      'progress': 0.0,
      'localPath': '',
    },
    {
      'name': 'Mistral 7B (Slice 4)',
      'description': 'Final pipeline chunk of Mistral.',
      'size': '4.1 GB',
      'url': 'https://huggingface.co/dummy/mistral.onnx',
      'isDownloaded': false,
      'isDownloading': false,
      'progress': 0.0,
      'localPath': '',
    },
  ];

  String? _loadedModelPath;

  Future<void> _downloadModel(int index) async {
    final model = _models[index];
    if (model['url'] == 'default') return;

    setState(() {
      model['isDownloading'] = true;
      model['progress'] = 0.0;
    });

    try {
      final appDocDir = await getApplicationDocumentsDirectory();
      final savePath = '${appDocDir.path}/${model['name']}.onnx';

      // Mocking the download for the MVP since we don't have actual 2GB URLs hosted
      // In production: await _dio.download(model['url'], savePath, onReceiveProgress: ...)
      for (int i = 0; i <= 10; i++) {
        await Future.delayed(const Duration(milliseconds: 300));
        setState(() {
          model['progress'] = i / 10;
        });
      }

      // Load a valid minimal ONNX file from assets instead of a dummy broken byte array
      final ByteData data = await rootBundle.load('assets/models/minimal.onnx');
      final List<int> bytes = data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
      final file = File(savePath);
      await file.writeAsBytes(bytes);

      setState(() {
        model['isDownloading'] = false;
        model['isDownloaded'] = true;
        model['localPath'] = savePath;
      });

      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('${model['name']} downloaded!')),
        );
      }
    } catch (e) {
      setState(() {
        model['isDownloading'] = false;
      });
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Download failed: $e')),
        );
      }
    }
  }

  Future<void> _loadModel(String path, String modelId) async {
    try {
      // Trigger the Seamless Device Distribution orchestrator protocol in Rust
      await triggerModelDistribution(modelId: modelId);
      
      setState(() {
        _loadedModelPath = path;
      });
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Model loaded! Distribution propagating through Mesh...')),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Failed to trigger distribution.')),
        );
      }
    }
  }


  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: const Color(0xFF1A1A1A),
      appBar: AppBar(
        title: const Text('Open Source Model Hub'),
        backgroundColor: const Color(0xFF2A2A2A),
        elevation: 0,
      ),
      body: ListView.builder(
        padding: const EdgeInsets.all(16),
        itemCount: _models.length,
        itemBuilder: (context, index) {
          final model = _models[index];
          final isLoaded = _loadedModelPath == model['localPath'];

          return Card(
            color: const Color(0xFF2A2A2A),
            margin: const EdgeInsets.only(bottom: 16),
            shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      Text(
                        model['name'],
                        style: const TextStyle(
                          color: Colors.white,
                          fontSize: 18,
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                      Text(
                        model['size'],
                        style: const TextStyle(color: Colors.grey),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Text(
                    model['description'],
                    style: const TextStyle(color: Colors.white70),
                  ),
                  const SizedBox(height: 16),
                  if (model['isDownloading'])
                    Column(
                      children: [
                        LinearProgressIndicator(
                          value: model['progress'],
                          backgroundColor: Colors.grey[800],
                          valueColor: const AlwaysStoppedAnimation<Color>(Colors.greenAccent),
                        ),
                        const SizedBox(height: 8),
                        Text(
                          '${(model['progress'] * 100).toStringAsFixed(1)}%',
                          style: const TextStyle(color: Colors.greenAccent),
                        ),
                      ],
                    )
                  else
                    Row(
                      mainAxisAlignment: MainAxisAlignment.end,
                      children: [
                        if (!model['isDownloaded'])
                          ElevatedButton.icon(
                            icon: const Icon(Icons.download),
                            label: const Text('Download'),
                            style: ElevatedButton.styleFrom(
                              backgroundColor: Colors.blueAccent,
                              foregroundColor: Colors.white,
                            ),
                            onPressed: () => _downloadModel(index),
                          )
                        else
                          ElevatedButton.icon(
                            icon: Icon(isLoaded ? Icons.check_circle : Icons.memory),
                            label: Text(isLoaded ? 'Active in Mesh' : 'Load into Mesh'),
                            style: ElevatedButton.styleFrom(
                              backgroundColor: isLoaded ? Colors.green : Colors.purpleAccent,
                              foregroundColor: Colors.white,
                            ),
                            onPressed: isLoaded
                              ? null
                              : () => _loadModel(model['localPath'], model['id']),
                          ),
                      ],
                    ),
                ],
              ),
            ),
          );
        },
      ),
    );
  }
}
