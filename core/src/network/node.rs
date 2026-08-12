use libp2p::{
    gossipsub, mdns, noise, swarm::SwarmEvent, tcp, yamux, Swarm, SwarmBuilder,
};
use std::error::Error;
use std::time::Duration;


use super::{MeshBehavior, MeshBehaviorEvent};
use crate::network::discovery::PeerHealth;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct MeshNode {
    pub swarm: Swarm<MeshBehavior>,
}

impl MeshNode {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let swarm = SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                let message_id_fn = |message: &gossipsub::Message| {
                    let mut s = DefaultHasher::new();
                    message.data.hash(&mut s);
                    gossipsub::MessageId::from(s.finish().to_string())
                };

                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(10))
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .message_id_fn(message_id_fn)
                    .build()
                    .expect("Valid config");

                let mut gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .expect("Correct configuration");

                let topic = gossipsub::IdentTopic::new("ai-mesh-inference");
                gossipsub.subscribe(&topic).unwrap();

                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;

                let local_peer_id = key.public().to_peer_id();
                let store = libp2p::kad::store::MemoryStore::new(local_peer_id);
                let kademlia = libp2p::kad::Behaviour::new(local_peer_id, store);

                let identify_config = libp2p::identify::Config::new("/ai-mesh/1.0.0".to_string(), key.public());
                let identify = libp2p::identify::Behaviour::new(identify_config);

                let autonat_config = libp2p::autonat::Config::default();
                let autonat = libp2p::autonat::Behaviour::new(local_peer_id, autonat_config);

                Ok(MeshBehavior { gossipsub, mdns, kademlia, identify, autonat })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        Ok(Self { swarm })
    }

    pub async fn start(
        &mut self,
        port: u16,
        peer_tx: mpsc::UnboundedSender<String>,
        peer_state: Arc<Mutex<Vec<PeerHealth>>>,
        cancel: CancellationToken,
    ) -> Result<(), Box<dyn Error>> {
        let addr = format!("/ip4/0.0.0.0/tcp/{}", port).parse()?;
        self.swarm.listen_on(addr)?;
        
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    println!("MeshNode libp2p swarm loop cancelled gracefully.");
                    break Ok(());
                }
                event = self.swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(MeshBehaviorEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, multiaddr) in list {
                            println!("mDNS discovered a new peer: {peer_id}");
                            self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            
                            // Send peer ID to Flutter UI stream
                            let _ = peer_tx.send(peer_id.to_string());
                            
                            // Update shared state
                            if let Ok(mut peers) = peer_state.lock() {
                                if !peers.iter().any(|p| p.peer_id == peer_id.to_string()) {
                                    peers.push(PeerHealth::new(
                                        peer_id.to_string(),
                                        multiaddr.to_string(),
                                        Duration::from_millis(0), // Ping latency would be measured separately
                                        true
                                    ));
                                }
                            }
                        }
                    },
                    SwarmEvent::Behaviour(MeshBehaviorEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _multiaddr) in list {
                            println!("mDNS discover peer has expired: {peer_id}");
                            self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                            
                            // Mark as dead in shared state
                            if let Ok(mut peers) = peer_state.lock() {
                                if let Some(peer) = peers.iter_mut().find(|p| p.peer_id == peer_id.to_string()) {
                                    peer.is_alive = false;
                                }
                            }
                        }
                    },
                    SwarmEvent::Behaviour(MeshBehaviorEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    })) => println!(
                            "Got message: '{}' with id: {id} from peer: {peer_id}",
                            String::from_utf8_lossy(&message.data),
                        ),
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Local node is listening on {address}");
                    }
                    _ => {}
                }
            }
        }
    }
}

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use futures::stream::StreamExt;
