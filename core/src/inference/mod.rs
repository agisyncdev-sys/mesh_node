pub mod tensor;

use std::error::Error;
use std::path::Path;

use ndarray::Array;
use ort::{inputs, session::Session, value::Tensor};

use tensor::MeshTensor;

/// Wraps an ONNX Runtime inference session.
///
/// On construction the ORT environment is initialised once (thread-safe global)
/// and the model file is loaded into a session backed by the pre-extracted
/// ORT 1.17.1 DLL (path baked in at compile time via `build.rs`).
pub struct InferenceEngine {
    session: Session,
    /// Cached first input/output name for diagnostics / run() calls.
    input_name: String,
    output_name: String,
}

impl InferenceEngine {
    /// Load an ONNX model from `model_path` and open an inference session.
    ///
    /// `ort` uses `ORT_DYLIB_PATH` (emitted by `build.rs`) to locate
    /// `onnxruntime.dll` via `LoadLibrary` at runtime.
    pub fn new(model_path: &str) -> Result<Self, Box<dyn Error>> {
        // init() returns bool (true = first init, false = already done). No ? needed.
        ort::init()
            .with_name("mesh_core")
            .commit();

        // Build the session with 1 intra-op thread — lightweight for edge/mobile.
        let session = Session::builder()?
            .with_intra_threads(1)?
            .commit_from_file(Path::new(model_path))?;

        // .inputs() / .outputs() are methods in rc.13, not public fields.
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name())
            .unwrap_or("input")
            .to_string();

        let output_name = session
            .outputs()
            .first()
            .map(|o| o.name())
            .unwrap_or("output")
            .to_string();

        println!(
            "[InferenceEngine] ONNX Runtime loaded. Model: {}\n  Input : [{}]\n  Output: [{}]",
            model_path, input_name, output_name
        );

        Ok(Self { session, input_name, output_name })
    }

    /// Log model graph metadata to stdout (useful for debugging / telemetry).
    pub fn print_model_info(&self) {
        println!(
            "[InferenceEngine] inputs : {:?}",
            self.session.inputs().iter().map(|i| i.name()).collect::<Vec<_>>()
        );
        println!(
            "[InferenceEngine] outputs: {:?}",
            self.session.outputs().iter().map(|o| o.name()).collect::<Vec<_>>()
        );
    }

    /// Run a single forward pass.
    ///
    /// * Converts `input` into an ndarray, wraps it in an `ort::Tensor`.
    /// * Runs the ORT session.
    /// * Extracts the first output tensor back into a `MeshTensor`.
    pub fn execute_forward_pass(
        &mut self,
        input: &MeshTensor,
    ) -> Result<MeshTensor, Box<dyn Error>> {
        let shape: Vec<usize> = input.shape.clone();
        let array = Array::from_shape_vec(ndarray::IxDyn(&shape), input.data.clone())?;

        // Wrap the ndarray in an ORT Tensor value (owned).
        let ort_tensor = Tensor::from_array(array)?;

        // inputs! macro returns Vec<(Cow<str>, SessionInputValue)> — no ? on it.
        let input_name: &str = &self.input_name;
        let outputs = self.session.run(inputs![input_name => ort_tensor])?;

        // try_extract_tensor returns (&Shape, &[ElementType]) as a tuple.
        let output_view = outputs[self.output_name.as_str()]
            .try_extract_tensor::<f32>()?;

        // .0 = &ort::value::Shape (implements Deref<[i64]>), .1 = &[f32]
        let out_shape: Vec<usize> = output_view.0.iter().map(|&d| d as usize).collect();
        let out_data: Vec<f32> = output_view.1.to_vec();

        Ok(MeshTensor::new(out_shape, out_data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the real ORT session loads `minimal.onnx` and runs a forward pass.
    /// The model is an Identity node (opset 18) so output must equal input exactly.
    #[test]
    fn test_inference_forward_pass() {
        let mut engine = InferenceEngine::new("minimal.onnx")
            .expect("Failed to initialise InferenceEngine with minimal.onnx");

        engine.print_model_info();

        // 42.0 as a mathematical placeholder for a tokenised word embedding.
        let input_data = vec![42.0f32];
        let input_tensor = MeshTensor::new(vec![1, 1], input_data.clone());

        let output_tensor = engine
            .execute_forward_pass(&input_tensor)
            .expect("Forward pass failed");

        println!("Output shape : {:?}", output_tensor.shape);
        println!("Output values: {:?}", output_tensor.data);

        // Identity model: output must equal input.
        assert_eq!(output_tensor.shape, vec![1, 1]);
        assert_eq!(output_tensor.data, input_data);
    }

    /// Verifies multi-element batches work correctly with the Identity model.
    #[test]
    fn test_batch_forward_pass() {
        let mut engine = InferenceEngine::new("minimal.onnx")
            .expect("Failed to initialise InferenceEngine");

        // Batch of 4 scalar embeddings packed as [4, 1].
        let input_data = vec![1.0f32, 2.0, 3.0, 4.0];
        let input_tensor = MeshTensor::new(vec![4, 1], input_data.clone());

        let output = engine
            .execute_forward_pass(&input_tensor)
            .expect("Batch forward pass failed");

        assert_eq!(output.shape, vec![4, 1]);
        assert_eq!(output.data, input_data);
    }
}
