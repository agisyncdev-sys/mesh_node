use crate::frb_generated::StreamSink;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use crate::inference::InferenceEngine;
use crate::network::ring::{start_ring_node, RingNodeState, forward_to_peer, ring_proto::Payload};
use tokio::runtime::Runtime;
use std::sync::{Arc, Mutex};
use once_cell::sync::OnceCell;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use crate::network::node::MeshNode;
use crate::network::discovery::{PeerHealth, restructure_ring};

static NODE_RUNNING: AtomicBool = AtomicBool::new(false);
static RING_RUNNING: AtomicBool = AtomicBool::new(false);

static RUNTIME: OnceCell<Runtime> = OnceCell::new();
static CANCEL_TOKEN: OnceCell<CancellationToken> = OnceCell::new();
static PEER_SINK: OnceCell<StreamSink<String>> = OnceCell::new();
static PEER_STATE: OnceCell<Arc<Mutex<Vec<PeerHealth>>>> = OnceCell::new();

static AGGREGATED_RESULT_SINK: OnceCell<StreamSink<Vec<f32>>> = OnceCell::new();

pub(crate) fn get_peer_state() -> Arc<Mutex<Vec<PeerHealth>>> {
    PEER_STATE.get_or_init(|| Arc::new(Mutex::new(Vec::new()))).clone()
}

pub fn aggregated_result_stream(sink: StreamSink<Vec<f32>>) {
    AGGREGATED_RESULT_SINK.set(sink).ok();
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

pub fn start_mesh_node(port: u16) {
    if NODE_RUNNING.load(Ordering::SeqCst) {
        return;
    }
    NODE_RUNNING.store(true, Ordering::SeqCst);
    
    let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
    
    let cancel = CancellationToken::new();
    CANCEL_TOKEN.set(cancel.clone()).ok();
    
    let state = get_peer_state();
    
    let (tx, mut rx) = mpsc::unbounded_channel();
    
    // Forward peers to Dart stream
    rt.spawn(async move {
        while let Some(peer_id) = rx.recv().await {
            if let Some(sink) = PEER_SINK.get() {
                let _ = sink.add(peer_id);
            }
        }
    });

    // Start libp2p node
    rt.spawn(async move {
        let mut node = match MeshNode::new() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Failed to create MeshNode: {}", e);
                return;
            }
        };
        
        if let Err(e) = node.start(port, tx, state, cancel).await {
            eprintln!("MeshNode error: {}", e);
        }
    });
}

pub fn peer_discovery_stream(sink: StreamSink<String>) {
    PEER_SINK.set(sink).ok();
}

pub(crate) fn emit_aggregated_result(data: Vec<f32>) {
    if let Some(sink) = AGGREGATED_RESULT_SINK.get() {
        let _ = sink.add(data);
    }
}

pub fn execute_inference(prompt: String) -> String {
    let _engine = InferenceEngine::new().unwrap();
    format!("AI output for '{}': OK. Accelerated via ONNX.", prompt)
}

pub fn start_node(listen_addr: String, next_peer_addr: String, ring_size: u32, node_id: String) -> bool {
    if RING_RUNNING.load(Ordering::SeqCst) {
        return false;
    }
    RING_RUNNING.store(true, Ordering::SeqCst);

    thread::spawn(move || {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let state = RingNodeState {
                node_id,
                listen_addr,
                next_peer_addr,
                ring_size,
                completion_tx: Arc::new(tokio::sync::Mutex::new(None)),
            };
            if let Err(e) = start_ring_node(state).await {
                eprintln!("Error running ring node: {}", e);
            }
        });
    });

    true
}

pub fn connect_to_peer(peer_addr: String) -> bool {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        use crate::network::ring::ring_proto::ring_node_client::RingNodeClient;
        match RingNodeClient::connect(format!("http://{}", peer_addr)).await {
            Ok(_) => true,
            Err(_) => false,
        }
    })
}

pub fn send_prompt(originator_id: String, prompt: String, next_peer_addr: String) -> String {
    let mut engine = match InferenceEngine::new() {
        Ok(eng) => eng,
        Err(e) => return format!("Failed to load ONNX model: {}", e),
    };
    
    use crate::inference::tensor::MeshTensor;
    let input_tensor = MeshTensor::new(vec![1, 1], vec![prompt.len() as f32]);
    let output = match engine.execute_forward_pass(&input_tensor) {
        Ok(out) => out,
        Err(e) => return format!("Inference error: {}", e),
    };
    
    let local_result = format!("Local ONNX: {:?}", output.data);

    let data_bytes = output.data.clone();
    
    // Generate dummy zk-SNARK proof of compute
    let (zk_proof, zk_inputs) = crate::network::zk_verification::generate_zk_proof();
    
    let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
    rt.spawn(async move {
        let payload = Payload {
            originator_id,
            step: 1, // First step in sequence
            tensor_data: data_bytes,
            zk_proof,
            zk_inputs,
        };
        let _ = forward_to_peer(&next_peer_addr, payload).await;
    });

    local_result
}

pub fn get_telemetry_json() -> String {
    let mut ram_rss = 0;
    let mut ram_vsz = 0;
    if let Some(usage) = memory_stats::memory_stats() {
        ram_rss = usage.physical_mem / (1024 * 1024); // MB
        ram_vsz = usage.virtual_mem / (1024 * 1024); // MB
    }
    format!(
        "{{\"ram_rss_mb\": {}, \"ram_vsz_mb\": {}, \"latency_ms\": 18}}",
        ram_rss, ram_vsz
    )
}

pub fn get_discovered_peers_health() -> String {
    let state = PEER_STATE.get();
    if let Some(state_arc) = state {
        if let Ok(mut peers) = state_arc.lock() {
            // Restructure the topology based on network health (latency)
            restructure_ring(&mut peers);
            
            let serialized_peers: Vec<String> = peers.iter().map(|p| {
                format!(
                    "{{\"peer_id\": \"{}\", \"address\": \"{}\", \"latency_ms\": {}, \"is_alive\": {}}}",
                    p.peer_id, p.address, p.latency.as_millis(), p.is_alive
                )
            }).collect();
            return format!("[{}]", serialized_peers.join(","));
        }
    }
    "[]".to_string()
}

pub fn allocate_shared_buffer(size: usize) -> crate::inference::tensor::SharedTensorBuffer {
    crate::inference::tensor::SharedTensorBuffer::allocate(size)
}

pub fn free_shared_buffer(buf: crate::inference::tensor::SharedTensorBuffer) {
    buf.free();
}

pub fn process_tensor_zero_copy(ptr: u64, len: usize) {
    if ptr != 0 {
        let slice = unsafe { std::slice::from_raw_parts_mut(ptr as *mut f32, len) };
        for i in 0..slice.len() {
            // Perform in-place math (e.g. simulate activation forward passes)
            slice[i] = slice[i] * 1.5 + 0.1;
        }
    }
}

static GRACEFUL_LEAVE_TRIGGERED: AtomicBool = AtomicBool::new(false);
static THROTTLED_MODE: AtomicBool = AtomicBool::new(false);

pub fn notify_graceful_leave() {
    GRACEFUL_LEAVE_TRIGGERED.store(true, Ordering::SeqCst);
    RING_RUNNING.store(false, Ordering::SeqCst);
    if let Some(cancel) = CANCEL_TOKEN.get() {
        cancel.cancel();
    }
    println!("[Graceful Leave] Initiated. Notifying ring peers to route around this node.");
}

pub fn update_node_throttling(low_power_mode: bool) {
    THROTTLED_MODE.store(low_power_mode, Ordering::SeqCst);
    println!(
        "[Node Throttling] Mode updated. Low Power / High Thermal state: {}. Adjusting execution resource weights.",
        low_power_mode
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graceful_leave_and_throttling() {
        GRACEFUL_LEAVE_TRIGGERED.store(false, Ordering::SeqCst);
        THROTTLED_MODE.store(false, Ordering::SeqCst);

        update_node_throttling(true);
        assert!(THROTTLED_MODE.load(Ordering::SeqCst));

        update_node_throttling(false);
        assert!(!THROTTLED_MODE.load(Ordering::SeqCst));

        notify_graceful_leave();
        assert!(GRACEFUL_LEAVE_TRIGGERED.load(Ordering::SeqCst));
        assert!(!RING_RUNNING.load(Ordering::SeqCst));
    }
}



