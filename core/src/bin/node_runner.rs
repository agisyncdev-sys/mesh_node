use std::error::Error;
use std::sync::Arc;
use tokio::sync::Mutex;
use rust_lib_mesh_ui::network::ring::{start_ring_node, RingNodeState, forward_to_peer, ring_proto::Payload};
use rust_lib_mesh_ui::inference::InferenceEngine;
use rust_lib_mesh_ui::inference::tensor::MeshTensor;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        println!("Usage: node_runner <node_id> <listen_port> <next_peer_port> <ring_size> [prompt]");
        return Ok(());
    }

    let node_id = args[1].clone();
    let listen_port = args[2].clone();
    let next_port = args[3].clone();
    let ring_size: u32 = args[4].parse().unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let completion_tx = if node_id == "Node-A" {
        Some(tx)
    } else {
        None
    };

    let state = RingNodeState {
        node_id: node_id.clone(),
        listen_addr: format!("127.0.0.1:{}", listen_port),
        next_peer_addr: format!("127.0.0.1:{}", next_port),
        ring_size,
        completion_tx: Arc::new(Mutex::new(completion_tx)),
    };

    let server_state = state.clone();
    tokio::spawn(async move {
        let _ = start_ring_node(server_state).await;
    });

    println!("Node {} listening on 127.0.0.1:{}", node_id, listen_port);

    // Wait a moment for server to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    if args.len() >= 6 {
        // We are triggering the prompt injection
        let prompt = args[5].clone();
        println!("Node {} executing local ONNX inference on prompt: '{}'", node_id, prompt);
        
        let mut engine = InferenceEngine::new_default()?;
        let input_tensor = MeshTensor::new(vec![1, 1], vec![prompt.len() as f32]);
        let output = engine.execute_forward_pass(&input_tensor)?;
        
        let result_str = format!("Result: {:?}", output.data);
        println!("Local ONNX Inference Result: {}", result_str);
        
        let (zk_proof, zk_inputs) = rust_lib_mesh_ui::network::zk_verification::generate_zk_proof();
        
        let payload = Payload {
            originator_id: node_id.clone(),
            step: 1,
            tensor_data: vec![0.5, 1.5, 2.5],
            zk_proof,
            zk_inputs,
        };

        if let Err(e) = forward_to_peer(&state.next_peer_addr, payload).await {
            eprintln!("Forward error: {}", e);
        }
    }

    if node_id == "Node-A" {
        // Wait for the message to return
        if let Some(payload) = rx.recv().await {
            println!("SUCCESS: Ring All-Reduce completed! Data returned to originator: {:?}", payload.tensor_data);
        }
    } else {
        // Keep running to process requests
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }

    Ok(())
}
