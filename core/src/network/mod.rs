pub mod node;
pub mod ring;
pub mod zk_verification;
pub mod discovery;

use libp2p::{
    gossipsub, mdns, swarm::NetworkBehaviour, kad, identify, autonat
};

#[derive(NetworkBehaviour)]
pub struct MeshBehavior {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub autonat: autonat::Behaviour,
}
