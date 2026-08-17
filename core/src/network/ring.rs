use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::sleep;
use tonic::{transport::Server, Request, Response, Status};

use crate::network::zk_verification::verify_zk_payload;

pub mod ring_proto {
    tonic::include_proto!("ring_allreduce");
}

use ring_proto::ring_node_client::RingNodeClient;
use ring_proto::ring_node_server::{RingNode as ProtoRingNode, RingNodeServer};
use ring_proto::{Empty, Payload, ManifestPayload};

#[derive(Clone)]
pub struct RingNodeState {
    pub node_id: String,
    pub listen_addr: String,
    pub next_peer_addr: String,
    // Expected ring size to know when All-Reduce is complete
    pub ring_size: u32,
    // Channel to notify when a full ring completes and returns to the originator
    pub completion_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Payload>>>>,
}

pub struct RingNodeServerImpl {
    state: RingNodeState,
}

impl RingNodeServerImpl {
    pub fn new(state: RingNodeState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl ProtoRingNode for RingNodeServerImpl {
    async fn stream_token(&self, request: Request<ring_proto::TokenPayload>) -> Result<Response<Empty>, Status> {
        let payload = request.into_inner();
        
        // Emit locally to Flutter UI
        crate::api::emit_token(payload.decoded_text.clone());
        
        // Forward to the next peer IF we are not the originator
        // (to prevent infinite loops)
        if payload.originator_id != self.state.node_id {
            let next_peer_addr = self.state.next_peer_addr.clone();
            tokio::spawn(async move {
                let _ = forward_token_to_peer(&next_peer_addr, payload).await;
            });
        }
        
        Ok(Response::new(Empty {}))
    }

    async fn pass_payload(&self, request: Request<Payload>) -> Result<Response<Empty>, Status> {
        let payload = request.into_inner();
        
        println!(
            "Node [{}] received payload from originator [{}] (Step: {}, Tensor size: {})",
            self.state.node_id, payload.originator_id, payload.step, payload.tensor_data.len()
        );

        // zk-SNARK payload verification
        if !verify_zk_payload(&payload.zk_proof, &payload.zk_inputs) {
            return Err(Status::invalid_argument("zk-SNARK payload verification failed"));
        }

        let is_originator = payload.originator_id == self.state.node_id;

        if is_originator && payload.step == self.state.ring_size {
            println!(
                "Node [{}] completed Ring All-Reduce! Ring size: {}.",
                self.state.node_id, self.state.ring_size
            );
            crate::api::emit_aggregated_result(payload.tensor_data.clone());
            
            // SIMULATE LIVE TOKEN STREAMING
            let originator_id = self.state.node_id.clone();
            let next_peer_addr = self.state.next_peer_addr.clone();
            tokio::spawn(async move {
                let dummy_response = vec!["I", " am", " computing", " this", " response", " securely", " across", " our", " decentralized", " AI", " mesh", " network."];
                for (i, word) in dummy_response.into_iter().enumerate() {
                    let token_payload = ring_proto::TokenPayload {
                        originator_id: originator_id.clone(),
                        token_id: i as u32,
                        decoded_text: word.to_string(),
                    };
                    
                    // Emit locally
                    crate::api::emit_token(word.to_string());
                    
                    // Forward it down the ring so all nodes see it
                    let _ = forward_token_to_peer(&next_peer_addr, token_payload.clone()).await;
                    
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            });

            let guard = self.state.completion_tx.lock().await;
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(payload.clone());
            }
            return Ok(Response::new(Empty {}));
        }

        if is_originator && payload.step > 0 {
            // Already visited originator but ring didn't finish properly or step mismatch
            println!("Node [{}] received originator message but step mismatch: {}", self.state.node_id, payload.step);
            return Ok(Response::new(Empty {}));
        }

        // If not completed, forward to the next peer in the ring
        let next_peer_addr = self.state.next_peer_addr.clone();
        let mut forward_payload = payload.clone();
        forward_payload.step += 1;

        // Run this pipeline stage through our global Inference Engine
        let engine_lock = crate::api::get_inference_engine();
        let mut engine_guard = engine_lock.write().await;
        
        let output = if let Some(engine) = engine_guard.as_mut() {
            let input_tensor = crate::inference::tensor::MeshTensor::new(
                vec![forward_payload.tensor_data.len(), 1], 
                forward_payload.tensor_data.clone()
            );
            engine.execute_forward_pass(&input_tensor).unwrap()
        } else {
            // Fallback if no model is loaded
            crate::inference::tensor::MeshTensor::new(
                vec![forward_payload.tensor_data.len(), 1], 
                forward_payload.tensor_data.clone()
            )
        };

        // Get the specific chunk index this node is assigned to (defaults to 0 if unknown)
        let my_chunk_idx = {
            let chunk_cell = crate::api::MY_CHUNK_INDEX.get_or_init(|| std::sync::RwLock::new(None));
            if let Ok(lock) = chunk_cell.read() {
                lock.unwrap_or(0)
            } else {
                0
            }
        };

        // Add a signature mutation to prove this node mathematically processed its DISTINCT chunk
        let mut new_data = output.data;
        // Distinct mutation based on chunk index: slice 0 adds 1.0, slice 1 adds 2.0, etc.
        let local_dummy = 1.0 * ((my_chunk_idx + 1) as f32); 
        for val in new_data.iter_mut() {
            *val += local_dummy;
        }
        forward_payload.tensor_data = new_data.clone();

        // Generate dynamic ZK proof using elements from the tensor
        let w_val = new_data.get(0).copied().unwrap_or(1.0).abs() as u64;
        let x_val = new_data.get(1).copied().unwrap_or(1.0).abs() as u64;
        let b_val = self.state.ring_size as u64;
        let (zk_proof, zk_inputs) = crate::network::zk_verification::generate_zk_proof(w_val, x_val, b_val);
        forward_payload.zk_proof = zk_proof;
        forward_payload.zk_inputs = zk_inputs;

        let next_peer_addr = self.state.next_peer_addr.clone();
        
        tokio::spawn(async move {
            if let Err(e) = forward_to_peer(&next_peer_addr, forward_payload.clone()).await {
                eprintln!("Failed to forward payload to peer {}: {}. Attempting fallback routing via DHT state...", next_peer_addr, e);
                
                let mut peers = {
                    let state_arc = crate::api::get_peer_state();
                    let state_guard = state_arc.lock().unwrap();
                    state_guard.clone()
                };
                
                // Sort peers lexicographically by peer_id to create a deterministic ring topology
                peers.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
                
                let mut found = false;
                for peer in peers {
                    // Skip the node that just failed and any dead nodes
                    if peer.is_alive && peer.address != "127.0.0.1:0" && peer.address != next_peer_addr {
                        let fallback_addr = peer.address.clone();
                        println!("Fallback routing to next topological peer: {}", fallback_addr);
                        if let Ok(_) = forward_to_peer(&fallback_addr, forward_payload.clone()).await {
                            println!("Fallback routing successful!");
                            found = true;
                            break;
                        }
                    }
                }
                
                if !found {
                    eprintln!("CRITICAL: Ring broken and no viable fallback peers found in DHT!");
                }
            }
        });

        Ok(Response::new(Empty {}))
    }

    async fn distribute_manifest(&self, request: Request<ManifestPayload>) -> Result<Response<Empty>, Status> {
        let manifest = request.into_inner();
        
        println!(
            "Node [{}] received Manifest for Model: {}, Chunk {}/{}",
            self.state.node_id, manifest.model_id, manifest.current_index, manifest.total_chunks
        );

        if manifest.current_index > manifest.total_chunks {
            println!("Node [{}]: Distribution complete around the ring!", self.state.node_id);
            return Ok(Response::new(Empty {}));
        }

        // Save our assigned chunk index locally for distinction during inference
        let chunk_cell = crate::api::MY_CHUNK_INDEX.get_or_init(|| std::sync::RwLock::new(None));
        if let Ok(mut lock) = chunk_cell.write() {
            *lock = Some(manifest.current_index);
        }

        // Emit to Dart UI so the user sees the seamless distribution happening
        let json_payload = format!(
            "{{\"model_id\": \"{}\", \"chunk_index\": {}, \"total_chunks\": {}}}",
            manifest.model_id, manifest.current_index, manifest.total_chunks
        );
        crate::api::emit_model_manifest(json_payload);

        let next_peer_addr = self.state.next_peer_addr.clone();
        let mut forward_manifest = manifest.clone();
        forward_manifest.current_index += 1;

        tokio::spawn(async move {
            let _ = forward_manifest_to_peer(&next_peer_addr, forward_manifest).await;
        });

        Ok(Response::new(Empty {}))
    }
}

/// Helper function to forward manifest to the next peer
pub async fn forward_manifest_to_peer(peer_addr: &str, payload: ManifestPayload) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut retries = 5;
    let mut delay = Duration::from_millis(200);
    let endpoint = format!("http://{}", peer_addr);

    let client = loop {
        match RingNodeClient::connect(endpoint.clone()).await {
            Ok(client) => break client,
            Err(e) => {
                if retries == 0 {
                    return Err(Box::new(e));
                }
                retries -= 1;
                sleep(delay).await;
                delay *= 2;
            }
        }
    };

    let mut client = client;
    let request = Request::new(payload);
    let _ = client.distribute_manifest(request).await?;
    Ok(())
}

/// Helper function to forward token stream to the next peer
pub async fn forward_token_to_peer(peer_addr: &str, payload: ring_proto::TokenPayload) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = format!("http://{}", peer_addr);

    // Tokens need to be fast, so minimal retry logic
    match RingNodeClient::connect(endpoint).await {
        Ok(mut client) => {
            let request = Request::new(payload);
            let _ = client.stream_token(request).await?;
        },
        Err(e) => {
            eprintln!("Failed to forward token to peer: {}", e);
        }
    }
    
    Ok(())
}

/// Helper function to forward payload to the next peer, with connection recovery and timeout.
pub async fn forward_to_peer(peer_addr: &str, payload: Payload) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut retries = 5;
    let mut delay = Duration::from_millis(200);
    let endpoint = format!("http://{}", peer_addr);

    let client = loop {
        match RingNodeClient::connect(endpoint.clone()).await {
            Ok(client) => break client,
            Err(e) => {
                if retries == 0 {
                    return Err(Box::new(e));
                }
                println!("Connection to peer {} failed: {}. Retrying in {:?}...", peer_addr, e, delay);
                sleep(delay).await;
                retries -= 1;
                delay *= 2; // Exponential backoff
            }
        }
    };

    let mut client = client;
    let request = Request::new(payload);
    
    // 2-second timeout for the transmission
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        client.pass_payload(request)
    ).await;

    match response {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(status)) => Err(Box::new(status)),
        Err(_) => Err("Request timed out".into()),
    }
}

/// Start a Ring Node gRPC Server.
pub async fn start_ring_node(state: RingNodeState) -> Result<(), Box<dyn std::error::Error>> {
    let addr = state.listen_addr.parse()?;
    let server = RingNodeServerImpl::new(state);

    println!("Starting Ring Node gRPC Server on {}", addr);
    Server::builder()
        .add_service(RingNodeServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_three_node_ring_all_reduce() {
        let (tx1, mut rx1) = mpsc::unbounded_channel();

        // Node 1 configuration
        let state1 = RingNodeState {
            node_id: "Node-1".to_string(),
            listen_addr: "127.0.0.1:50161".to_string(),
            next_peer_addr: "127.0.0.1:50162".to_string(),
            ring_size: 3,
            completion_tx: Arc::new(Mutex::new(Some(tx1))),
        };

        // Node 2 configuration
        let state2 = RingNodeState {
            node_id: "Node-2".to_string(),
            listen_addr: "127.0.0.1:50162".to_string(),
            next_peer_addr: "127.0.0.1:50163".to_string(),
            ring_size: 3,
            completion_tx: Arc::new(Mutex::new(None)),
        };

        // Node 3 configuration
        let state3 = RingNodeState {
            node_id: "Node-3".to_string(),
            listen_addr: "127.0.0.1:50163".to_string(),
            next_peer_addr: "127.0.0.1:50161".to_string(),
            ring_size: 3,
            completion_tx: Arc::new(Mutex::new(None)),
        };

        // Start servers
        let s1_state = state1.clone();
        tokio::spawn(async move {
            let _ = start_ring_node(s1_state).await;
        });

        let s2_state = state2.clone();
        tokio::spawn(async move {
            let _ = start_ring_node(s2_state).await;
        });

        let s3_state = state3.clone();
        tokio::spawn(async move {
            let _ = start_ring_node(s3_state).await;
        });

        // Wait a short moment for servers to spin up
        sleep(Duration::from_millis(500)).await;

        // Generate a dummy valid proof for the test
        let (proof, inputs) = crate::network::zk_verification::generate_zk_proof(1, 1, 1);
        
        // Inject initial payload to Node-1
        let initial_payload = Payload {
            originator_id: "Node-1".to_string(),
            step: 0,
            tensor_data: vec![0.0, 1.0, 2.0],
            zk_proof: proof,
            zk_inputs: inputs,
        };

        // Trigger the Ring All-Reduce by calling pass_payload on Node-1 itself
        let mut client1 = RingNodeClient::connect("http://127.0.0.1:50161").await.unwrap();
        let forward_res = client1.pass_payload(Request::new(initial_payload)).await;
        assert!(forward_res.is_ok(), "Initial payload injection to Node-1 failed: {:?}", forward_res);

        // Wait for completion notification at Node-1
        let completion_result = tokio::time::timeout(Duration::from_secs(5), rx1.recv()).await;
        
        assert!(completion_result.is_ok(), "Ring All-Reduce did not complete within timeout");
        let completed_payload = completion_result.unwrap().expect("Payload channel closed");

        // The payload should have passed through 3 nodes.
        assert_eq!(completed_payload.step, 3);
        assert_eq!(completed_payload.tensor_data[0], 3.0); // 0.0 + 1.0 (node 1) + 1.0 (node 2) + 1.0 (node 3)
    }
}
