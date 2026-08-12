use ndarray::{Array, IxDyn};
use std::error::Error;

/// A utility wrapper to facilitate passing tensors between Dart (FFI) and Rust `ort`.
pub struct MeshTensor {
    pub shape: Vec<usize>,
    pub data: Vec<f32>, // We default to f32 for AI prototypes, can be genericized later
}

impl MeshTensor {
    pub fn new(shape: Vec<usize>, data: Vec<f32>) -> Self {
        Self { shape, data }
    }

    /// Converts the FFI-friendly representation into an ndarray for `ort`.
    pub fn to_ndarray(&self) -> Result<Array<f32, IxDyn>, Box<dyn Error>> {
        let array = Array::from_shape_vec(IxDyn(&self.shape), self.data.clone())?;
        Ok(array)
    }
}

/// A C-compatible shared memory buffer for zero-copy tensor pass-through.
#[derive(Debug)]
pub struct SharedTensorBuffer {
    pub ptr: u64, // Raw pointer cast to u64 for safe FFI bridging
    pub len: usize,
    pub capacity: usize,
}

#[flutter_rust_bridge::frb(ignore)]
impl SharedTensorBuffer {
    /// Allocates memory on the heap for f32 elements and returns the SharedTensorBuffer descriptor.
    pub fn allocate(size: usize) -> Self {
        let mut vec = vec![0.0f32; size];
        let ptr = vec.as_mut_ptr();
        let len = vec.len();
        let capacity = vec.capacity();
        std::mem::forget(vec); // Prevent deallocation of the vector memory
        Self {
            ptr: ptr as u64,
            len,
            capacity,
        }
    }

    /// Safely frees the allocated memory by reconstructing the Vector.
    pub fn free(self) {
        if self.ptr != 0 {
            unsafe {
                let _ = Vec::from_raw_parts(self.ptr as *mut f32, self.len, self.capacity);
            }
        }
    }

    /// Accesses the buffer contents as a read-only slice.
    pub fn as_slice(&self) -> &[f32] {
        if self.ptr == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr as *const f32, self.len) }
        }
    }

    /// Accesses the buffer contents as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        if self.ptr == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.ptr as *mut f32, self.len) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_tensor_buffer_alloc_free() {
        let mut buf = SharedTensorBuffer::allocate(100);
        assert_eq!(buf.len, 100);
        assert_ne!(buf.ptr, 0);

        // Modify elements in-place zero-copy
        let slice = buf.as_mut_slice();
        slice[0] = 42.0;
        slice[99] = 123.45;

        // Verify elements
        let read_slice = buf.as_slice();
        assert_eq!(read_slice[0], 42.0);
        assert_eq!(read_slice[99], 123.45);

        // Free memory
        buf.free();
    }
}
