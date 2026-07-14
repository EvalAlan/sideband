//! Deterministic outbound transport registration and route selection.

use std::net::SocketAddr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RouteEndpoint {
    Lan(SocketAddr),
    Bluetooth(crate::transport::bluetooth::BluetoothProperty),
    Tor,
    Opaque,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteCandidate {
    name: &'static str,
    priority: u16,
    reachable: bool,
    endpoint: RouteEndpoint,
}

impl RouteCandidate {
    pub(crate) fn new(name: &'static str, priority: u16, reachable: bool) -> Self {
        Self {
            name,
            priority,
            reachable,
            endpoint: RouteEndpoint::Opaque,
        }
    }

    pub(crate) fn with_endpoint(
        name: &'static str,
        priority: u16,
        reachable: bool,
        endpoint: RouteEndpoint,
    ) -> Self {
        Self {
            name,
            priority,
            reachable,
            endpoint,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) fn endpoint(&self) -> &RouteEndpoint {
        &self.endpoint
    }
}

#[derive(Default)]
pub(crate) struct TransportRegistry {
    candidates: Vec<RouteCandidate>,
}

impl TransportRegistry {
    pub(crate) fn register(&mut self, candidate: RouteCandidate) {
        self.candidates.retain(|old| old.name != candidate.name);
        self.candidates.push(candidate);
    }

    pub(crate) fn routes(&self) -> Vec<RouteCandidate> {
        let mut routes: Vec<_> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.reachable)
            .cloned()
            .collect();
        routes.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.name.cmp(b.name)));
        routes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_orders_reachable_routes_and_falls_back_to_tor() {
        let mut registry = TransportRegistry::default();
        registry.register(RouteCandidate::new("tor", 10, true));
        registry.register(RouteCandidate::new("bluetooth", 80, false));
        registry.register(RouteCandidate::new("lan", 100, true));
        assert_eq!(
            registry
                .routes()
                .iter()
                .map(|route| route.name())
                .collect::<Vec<_>>(),
            vec!["lan", "tor"]
        );
    }

    #[test]
    fn duplicate_registration_replaces_previous_reachability() {
        let mut registry = TransportRegistry::default();
        registry.register(RouteCandidate::new("lan", 100, false));
        registry.register(RouteCandidate::new("lan", 100, true));
        assert_eq!(registry.routes().len(), 1);
    }
}
