use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerHealth {
    pub peer_id: String,
    pub address: String,
    pub latency: Duration,
    pub is_alive: bool,
}

impl PeerHealth {
    pub fn new(peer_id: String, address: String, latency: Duration, is_alive: bool) -> Self {
        Self {
            peer_id,
            address,
            latency,
            is_alive,
        }
    }
}

/// Dynamically restructures the Ring Topology by sorting available peers based on connection latency (RTT).
/// Groups the lowest-latency, physically closest nodes next to each other to minimize Ring All-Reduce transit bottlenecks.
pub fn restructure_ring(peers: &mut [PeerHealth]) {
    // Only keep active/alive peers
    // Sort ascending by latency RTT
    peers.sort_by(|a, b| {
        if a.is_alive != b.is_alive {
            // Put dead peers at the end
            b.is_alive.cmp(&a.is_alive)
        } else {
            a.latency.cmp(&b.latency)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_latency_restructuring() {
        let mut peers = vec![
            PeerHealth::new("Node-Far".to_string(), "127.0.0.1:50063".to_string(), Duration::from_millis(250), true),
            PeerHealth::new("Node-Near".to_string(), "127.0.0.1:50061".to_string(), Duration::from_millis(15), true),
            PeerHealth::new("Node-Dead".to_string(), "127.0.0.1:50064".to_string(), Duration::from_millis(5), false),
            PeerHealth::new("Node-Mid".to_string(), "127.0.0.1:50062".to_string(), Duration::from_millis(85), true),
        ];

        // Restructure topology
        restructure_ring(&mut peers);

        // Verify correct order: lowest latency alive first, dead last
        assert_eq!(peers[0].peer_id, "Node-Near");
        assert_eq!(peers[1].peer_id, "Node-Mid");
        assert_eq!(peers[2].peer_id, "Node-Far");
        assert_eq!(peers[3].peer_id, "Node-Dead");
    }
}
