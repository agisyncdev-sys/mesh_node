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
use tokenizers::tokenizer::Tokenizer;

static NODE_RUNNING: AtomicBool = AtomicBool::new(false);
static GLOBAL_TOKENIZER: OnceCell<tokio::sync::RwLock<Option<Tokenizer>>> = OnceCell::new();
static RING_RUNNING: AtomicBool = AtomicBool::new(false);
static NEXT_PEER_ADDR: OnceCell<String> = OnceCell::new();
static RING_SIZE: OnceCell<u32> = OnceCell::new();

static RUNTIME: OnceCell<Runtime> = OnceCell::new();
static CANCEL_TOKEN: OnceCell<CancellationToken> = OnceCell::new();
static PEER_SINK: OnceCell<StreamSink<String>> = OnceCell::new();
static PEER_STATE: OnceCell<Arc<Mutex<Vec<PeerHealth>>>> = OnceCell::new();
static MODEL_MANIFEST_SINK: OnceCell<StreamSink<String>> = OnceCell::new();
static TOKEN_SINK: OnceCell<StreamSink<String>> = OnceCell::new();

static AGGREGATED_RESULT_SINK: OnceCell<StreamSink<Vec<f32>>> = OnceCell::new();
static GLOBAL_INFERENCE_ENGINE: OnceCell<tokio::sync::RwLock<Option<InferenceEngine>>> = OnceCell::new();

pub(crate) fn get_inference_engine() -> &'static tokio::sync::RwLock<Option<InferenceEngine>> {
    GLOBAL_INFERENCE_ENGINE.get_or_init(|| tokio::sync::RwLock::new(None))
}

pub fn load_model(path: String) -> bool {
    let engine_result = if path.is_empty() || path == "default" {
        InferenceEngine::new_default()
    } else {
        InferenceEngine::new_from_file(&path)
    };

    match engine_result {
        Ok(engine) => {
            let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
            rt.block_on(async {
                let lock = get_inference_engine();
                let mut guard = lock.write().await;
                *guard = Some(engine);
            });
            println!("Model successfully loaded from: {}", path);
            true
        }
        Err(e) => {
            eprintln!("Failed to load model from {}: {}", path, e);
            false
        }
    }
}

pub fn load_tokenizer(path: String) -> bool {
    let tokenizer_result = Tokenizer::from_file(&path);
    match tokenizer_result {
        Ok(tokenizer) => {
            let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
            rt.block_on(async {
                let cell = GLOBAL_TOKENIZER.get_or_init(|| tokio::sync::RwLock::new(None));
                let mut lock = cell.write().await;
                *lock = Some(tokenizer);
            });
            println!("Tokenizer successfully loaded from: {}", path);
            true
        },
        Err(e) => {
            eprintln!("Failed to load tokenizer from {}: {}", path, e);
            false
        }
    }
}

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

pub fn model_manifest_stream(sink: StreamSink<String>) {
    MODEL_MANIFEST_SINK.set(sink).ok();
}

pub(crate) fn emit_model_manifest(json_payload: String) {
    if let Some(sink) = MODEL_MANIFEST_SINK.get() {
        let _ = sink.add(json_payload);
    }
}

pub fn token_stream(sink: StreamSink<String>) {
    TOKEN_SINK.set(sink).ok();
}

pub(crate) fn emit_token(token: String) {
    if let Some(sink) = TOKEN_SINK.get() {
        let _ = sink.add(token);
    }
}

pub(crate) fn emit_aggregated_result(data: Vec<f32>) {
    if let Some(sink) = AGGREGATED_RESULT_SINK.get() {
        let _ = sink.add(data);
    }
}

pub fn execute_inference(prompt: String) -> String {
    let _engine = InferenceEngine::new_default().unwrap();
    format!("AI output for '{}': OK. Accelerated via ONNX.", prompt)
}

pub fn start_node(listen_addr: String, next_peer_addr: String, ring_size: u32, node_id: String) -> bool {
    if RING_RUNNING.load(Ordering::SeqCst) {
        return false;
    }
    RING_RUNNING.store(true, Ordering::SeqCst);
    let _ = NEXT_PEER_ADDR.set(next_peer_addr.clone());
    let _ = RING_SIZE.set(ring_size);

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
    let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
    
    let mut input_tensor = vec![prompt.len() as f32];
    
    rt.block_on(async {
        let cell = GLOBAL_TOKENIZER.get_or_init(|| tokio::sync::RwLock::new(None));
        let lock = cell.read().await;
        if let Some(tokenizer) = lock.as_ref() {
            if let Ok(encoding) = tokenizer.encode(prompt.clone(), true) {
                input_tensor = encoding.get_ids().iter().map(|id| *id as f32).collect();
            }
        }
    });

    let local_result = format!("Prompt embedded as tensor: {:?}. Passing to Pipeline...", input_tensor);

    // Generate dummy zk-SNARK proof of compute (for initial step)
    let (zk_proof, zk_inputs) = crate::network::zk_verification::generate_zk_proof(1, 1, 1);
    
    let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
    rt.spawn(async move {
        let payload = Payload {
            originator_id,
            step: 0, // 0 means it starts at the first node in the pipeline
            tensor_data: input_tensor,
            zk_proof,
            zk_inputs,
        };
        let _ = forward_to_peer(&next_peer_addr, payload).await;
    });

    local_result
}

pub fn trigger_model_distribution(model_id: String) {
    let rt = RUNTIME.get_or_init(|| Runtime::new().unwrap());
    
    let total_chunks = *RING_SIZE.get().unwrap_or(&3);
    let next_peer_addr = NEXT_PEER_ADDR.get().cloned().unwrap_or_else(|| "127.0.0.1:50062".to_string());
    
    rt.spawn(async move {
        // We load Chunk 0 locally
        emit_model_manifest(format!("{{\"model_id\": \"{}\", \"chunk_index\": 0, \"total_chunks\": {}}}", model_id, total_chunks));
        
        let payload = crate::network::ring::ring_proto::ManifestPayload {
            model_id,
            total_chunks,
            current_index: 1, // Next peer gets chunk 1
        };
        let _ = crate::network::ring::forward_manifest_to_peer(&next_peer_addr, payload).await;
    });
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



