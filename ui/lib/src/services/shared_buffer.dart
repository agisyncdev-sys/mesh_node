import 'dart:ffi';
import 'dart:typed_data';
import '../rust/api.dart/api.dart' as rust_api;
import '../rust/api.dart/inference/tensor.dart';

/// A wrapper around Rust's SharedTensorBuffer that exposes a zero-copy
/// Float32List interface for Dart/Flutter and handles strict memory management.
class SafeSharedBuffer {
  final SharedTensorBuffer _buffer;
  late final Float32List _dataView;
  bool _isDisposed = false;

  // Finalizer to ensure memory is released if the object is garbage-collected
  static final Finalizer<SharedTensorBuffer> _finalizer = Finalizer((buf) {
    rust_api.freeSharedBuffer(buf: buf);
  });

  SafeSharedBuffer._(this._buffer) {
    // Cast the raw u64 pointer address directly into a Dart FFI Pointer and convert to TypedList zero-copy
    final Pointer<Float> rawPtr =
        Pointer<Float>.fromAddress(_buffer.ptr.toInt());
    _dataView = rawPtr.asTypedList(_buffer.len.toInt());

    // Register this instance with the finalizer
    _finalizer.attach(this, _buffer, detach: this);
  }

  /// Allocates a new shared memory buffer of the given size (number of f32 elements).
  static Future<SafeSharedBuffer> allocate(int size) async {
    final sharedBuf =
        await rust_api.allocateSharedBuffer(size: BigInt.from(size));
    return SafeSharedBuffer._(sharedBuf);
  }

  /// Exposes direct, zero-copy read/write access to the raw Float32List buffer.
  Float32List get data {
    if (_isDisposed) {
      throw StateError('Cannot access data on a disposed SharedBuffer');
    }
    return _dataView;
  }

  /// Retrieves the raw transparent SharedTensorBuffer wrapper.
  SharedTensorBuffer get rawBuffer => _buffer;

  /// Retrieves the memory address pointer.
  int get address => _buffer.ptr.toInt();

  /// Retrieves the buffer size.
  int get length => _buffer.len.toInt();

  /// Explicitly frees the allocated heap memory to prevent memory leaks/OOMs.
  Future<void> dispose() async {
    if (_isDisposed) return;
    _isDisposed = true;
    _finalizer.detach(this);
    await rust_api.freeSharedBuffer(buf: _buffer);
  }
}
