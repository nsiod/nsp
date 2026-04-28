//! IPAM allocator for the WireGuard peer subnet.
//!
//! Plain data-structure over an `Ipv4Network`; no I/O, no async. The driver
//! builds one of these on spawn (seeding it from the persisted peers) and
//! calls [`Ipam::allocate`] / [`Ipam::release`] as peers come and go.
//!
//! The first host address (subnet `.1` by default) is reserved for the
//! server and therefore unavailable for peer allocation.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use ipnetwork::Ipv4Network;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpamError {
    #[error("subnet `{0}` is too small for even one peer address")]
    SubnetTooSmall(Ipv4Network),

    #[error("subnet has no more free host addresses")]
    Exhausted,

    #[error("address `{0}` is outside subnet `{1}`")]
    OutOfSubnet(Ipv4Addr, Ipv4Network),

    #[error("address `{0}` is reserved for the server")]
    Reserved(Ipv4Addr),

    #[error("address `{0}` is the subnet network address")]
    NetworkAddress(Ipv4Addr),

    #[error("address `{0}` is the subnet broadcast address")]
    BroadcastAddress(Ipv4Addr),
}

/// Stateful allocator. Keeps track of used and freed addresses in the subnet.
#[derive(Debug, Clone)]
pub struct Ipam {
    subnet: Ipv4Network,
    /// Subnet `.1` — the WG server address.
    server: Ipv4Addr,
    used: BTreeSet<u32>,
}

impl Ipam {
    /// Build an allocator for `subnet`, with the server pinned at the first
    /// host address.
    ///
    /// Pre-seed the allocator with every IP already leased to a peer via
    /// [`Ipam::with_used`]; the returned `Ipam` is the source of truth once
    /// constructed.
    pub fn new(subnet: Ipv4Network) -> Result<Self, IpamError> {
        let first = host_range(subnet)
            .ok_or(IpamError::SubnetTooSmall(subnet))?
            .0;
        Ok(Self {
            subnet,
            server: first,
            used: BTreeSet::new(),
        })
    }

    /// Pre-mark an IP as used. Idempotent.
    pub fn mark_used(&mut self, ip: Ipv4Addr) -> Result<(), IpamError> {
        self.check_in_range(ip)?;
        self.used.insert(u32::from(ip));
        Ok(())
    }

    /// Seed from an existing collection of leased peer IPs.
    pub fn with_used(mut self, ips: impl IntoIterator<Item = Ipv4Addr>) -> Result<Self, IpamError> {
        for ip in ips {
            self.mark_used(ip)?;
        }
        Ok(self)
    }

    /// Server (gateway) address — `.1` in a typical `/24`.
    pub fn server_addr(&self) -> Ipv4Addr {
        self.server
    }

    /// Configured subnet.
    pub fn subnet(&self) -> Ipv4Network {
        self.subnet
    }

    /// Take the next free host address. Returns [`IpamError::Exhausted`] when
    /// every usable address in the subnet is already taken.
    pub fn allocate(&mut self) -> Result<Ipv4Addr, IpamError> {
        let (first, last) =
            host_range(self.subnet).ok_or(IpamError::SubnetTooSmall(self.subnet))?;
        let server_u = u32::from(self.server);
        let first_u = u32::from(first);
        let last_u = u32::from(last);

        let mut candidate = first_u;
        while candidate <= last_u {
            if candidate == server_u || self.used.contains(&candidate) {
                candidate += 1;
                continue;
            }
            self.used.insert(candidate);
            return Ok(Ipv4Addr::from(candidate));
        }
        Err(IpamError::Exhausted)
    }

    /// Release a previously allocated IP. Idempotent: releasing an address
    /// that was never allocated is a no-op.
    pub fn release(&mut self, ip: Ipv4Addr) -> Result<(), IpamError> {
        self.check_in_range(ip)?;
        if ip == self.server {
            return Err(IpamError::Reserved(ip));
        }
        self.used.remove(&u32::from(ip));
        Ok(())
    }

    /// Returns `true` when `ip` is currently allocated to a peer.
    pub fn is_allocated(&self, ip: Ipv4Addr) -> bool {
        self.used.contains(&u32::from(ip))
    }

    fn check_in_range(&self, ip: Ipv4Addr) -> Result<(), IpamError> {
        if !self.subnet.contains(ip) {
            return Err(IpamError::OutOfSubnet(ip, self.subnet));
        }
        if ip == self.subnet.network() {
            return Err(IpamError::NetworkAddress(ip));
        }
        if ip == self.subnet.broadcast() {
            return Err(IpamError::BroadcastAddress(ip));
        }
        Ok(())
    }
}

/// First and last usable host addresses in `subnet`, or `None` when the
/// prefix is too wide to leave any usable host bits (a `/31` or `/32`).
fn host_range(subnet: Ipv4Network) -> Option<(Ipv4Addr, Ipv4Addr)> {
    let prefix = subnet.prefix();
    if prefix >= 31 {
        return None;
    }
    let network = u32::from(subnet.network());
    let broadcast = u32::from(subnet.broadcast());
    if broadcast <= network + 1 {
        return None;
    }
    Some((Ipv4Addr::from(network + 1), Ipv4Addr::from(broadcast - 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(s: &str) -> Ipv4Network {
        s.parse().expect("parse subnet")
    }

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().expect("parse ip")
    }

    #[test]
    fn allocates_sequentially_skipping_server() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).expect("new");
        assert_eq!(ipam.server_addr(), ip("10.66.66.1"));
        assert_eq!(ipam.allocate().unwrap(), ip("10.66.66.2"));
        assert_eq!(ipam.allocate().unwrap(), ip("10.66.66.3"));
        assert_eq!(ipam.allocate().unwrap(), ip("10.66.66.4"));
    }

    #[test]
    fn allocate_fills_holes_after_release() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        let a = ipam.allocate().unwrap(); // .2
        let b = ipam.allocate().unwrap(); // .3
        let c = ipam.allocate().unwrap(); // .4
        assert_eq!(a, ip("10.66.66.2"));
        assert_eq!(b, ip("10.66.66.3"));
        assert_eq!(c, ip("10.66.66.4"));

        ipam.release(b).unwrap();
        let next = ipam.allocate().unwrap();
        assert_eq!(next, ip("10.66.66.3"));
    }

    #[test]
    fn release_is_idempotent() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        let a = ipam.allocate().unwrap();
        ipam.release(a).unwrap();
        // Releasing again is a no-op.
        ipam.release(a).unwrap();
        assert!(!ipam.is_allocated(a));
    }

    #[test]
    fn release_rejects_out_of_subnet() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        let err = ipam.release(ip("10.77.0.5")).unwrap_err();
        assert!(matches!(err, IpamError::OutOfSubnet(_, _)));
    }

    #[test]
    fn release_rejects_server_address() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        let err = ipam.release(ip("10.66.66.1")).unwrap_err();
        assert!(matches!(err, IpamError::Reserved(_)));
    }

    #[test]
    fn exhaustion_returns_error() {
        // /29 gives us 10.0.0.0 - 10.0.0.7, usable hosts .1 - .6 (6 addrs),
        // .1 reserved for server -> 5 allocatable peer addresses.
        let mut ipam = Ipam::new(net("10.0.0.0/29")).unwrap();
        let allocations: Vec<_> = (0..5).map(|_| ipam.allocate().unwrap()).collect();
        assert_eq!(allocations.len(), 5);
        let err = ipam.allocate().unwrap_err();
        assert!(matches!(err, IpamError::Exhausted));
    }

    #[test]
    fn server_address_never_allocated() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        let server = ipam.server_addr();
        for _ in 0..10 {
            let allocated = ipam.allocate().unwrap();
            assert_ne!(allocated, server);
        }
    }

    #[test]
    fn pre_seeded_ips_are_skipped() {
        let used = vec![ip("10.66.66.2"), ip("10.66.66.3")];
        let mut ipam = Ipam::new(net("10.66.66.0/24"))
            .unwrap()
            .with_used(used)
            .unwrap();
        assert_eq!(ipam.allocate().unwrap(), ip("10.66.66.4"));
    }

    #[test]
    fn rejects_prefixes_too_wide() {
        let err = Ipam::new(net("10.0.0.0/31")).unwrap_err();
        assert!(matches!(err, IpamError::SubnetTooSmall(_)));
        let err = Ipam::new(net("10.0.0.0/32")).unwrap_err();
        assert!(matches!(err, IpamError::SubnetTooSmall(_)));
    }

    #[test]
    fn mark_used_prevents_reallocation() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        ipam.mark_used(ip("10.66.66.2")).unwrap();
        assert_eq!(ipam.allocate().unwrap(), ip("10.66.66.3"));
    }

    #[test]
    fn rejects_network_and_broadcast() {
        let mut ipam = Ipam::new(net("10.66.66.0/24")).unwrap();
        let err = ipam.mark_used(ip("10.66.66.0")).unwrap_err();
        assert!(matches!(err, IpamError::NetworkAddress(_)));
        let err = ipam.mark_used(ip("10.66.66.255")).unwrap_err();
        assert!(matches!(err, IpamError::BroadcastAddress(_)));
    }
}
