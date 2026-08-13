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
use ring_proto::{Empty, Payload};

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

        // Run this pipeline stage through our local Inference Engine
        let mut engine = crate::inference::InferenceEngine::new().unwrap();
        let input_tensor = crate::inference::tensor::MeshTensor::new(
            vec![forward_payload.tensor_data.len(), 1], 
            forward_payload.tensor_data.clone()
        );
        
        let output = engine.execute_forward_pass(&input_tensor).unwrap();

        // Add a signature mutation to prove this node mathematically processed it
        let mut new_data = output.data;
        let local_dummy = 1.0; 
        for val in new_data.iter_mut() {
            *val += local_dummy;
        }
        forward_payload.tensor_data = new_data;

        let next_peer_addr = self.state.next_peer_addr.clone();
        
        tokio::spawn(async move {
            if let Err(e) = forward_to_peer(&next_peer_addr, forward_payload.clone()).await {
                eprintln!("Failed to forward payload to peer {}: {}. Attempting fallback routing via DHT state...", next_peer_addr, e);
                
                let peers = {
                    let state_arc = crate::api::get_peer_state();
                    let state_guard = state_arc.lock().unwrap();
                    state_guard.clone()
                };
                
                let mut found = false;
                for peer in peers {
                    if peer.is_alive && peer.address != "127.0.0.1:0" {
                        let fallback_addr = peer.address.clone();
                        println!("Fallback routing to newly discovered peer: {}", fallback_addr);
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
        let (proof, inputs) = crate::network::zk_verification::generate_zk_proof();
        
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
