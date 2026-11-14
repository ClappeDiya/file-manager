//! IP allowlists from CIDR notation (`10.0.0.0/8`, `2001:db8::/32`, or a
//! single address). IPv4-mapped IPv6 peers (`::ffff:10.1.2.3`) match IPv4
//! ranges.

use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    net: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, Some(p)),
            None => (s, None),
        };
        let net: IpAddr = ip.parse().map_err(|_| format!("'{s}' is not an IP address or CIDR range"))?;
        let max = if net.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("'{s}' has an invalid prefix length"))?,
            None => max,
        };
        Ok(Self { net: mask(net, prefix), prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        ip.is_ipv4() == self.net.is_ipv4() && mask(ip, self.prefix) == self.net
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.net, self.prefix)
    }
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

fn mask(ip: IpAddr, prefix: u8) -> IpAddr {
    match canonical(ip) {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let m = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix.min(32)) };
            IpAddr::V4((bits & m).into())
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let m = if prefix == 0 { 0 } else { u128::MAX << (128 - prefix.min(128)) };
            IpAddr::V6((bits & m).into())
        }
    }
}

/// An allowlist; empty means every address is allowed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpAllowlist(Vec<Cidr>);

impl IpAllowlist {
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, String> {
        entries.iter().map(|e| e.as_ref()).filter(|e| !e.trim().is_empty()).map(Cidr::parse).collect::<Result<Vec<_>, _>>().map(Self)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn allows(&self, ip: IpAddr) -> bool {
        self.0.is_empty() || self.0.iter().any(|c| c.contains(ip))
    }

    pub fn ranges(&self) -> &[Cidr] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ipv4_and_ipv6_ranges() {
        let l = IpAllowlist::parse(&["10.0.0.0/8", "192.168.1.7", "2001:db8::/32", " "]).unwrap();
        assert!(l.allows(ip("10.255.0.1")));
        assert!(!l.allows(ip("11.0.0.1")));
        assert!(l.allows(ip("192.168.1.7")));
        assert!(!l.allows(ip("192.168.1.8")));
        assert!(l.allows(ip("2001:db8:1::5")));
        assert!(!l.allows(ip("2001:db9::5")));
        assert!(l.allows(ip("::ffff:10.1.2.3")), "IPv4-mapped peers match IPv4 ranges");
        assert_eq!(l.ranges()[0].to_string(), "10.0.0.0/8");
        // Host bits are cleared.
        assert_eq!(Cidr::parse("10.1.2.3/16").unwrap().to_string(), "10.1.0.0/16");
    }

    #[test]
    fn empty_allows_everything_and_bad_entries_fail() {
        assert!(IpAllowlist::default().allows(ip("8.8.8.8")));
        assert!(IpAllowlist::parse(&["0.0.0.0/0"]).unwrap().allows(ip("8.8.8.8")));
        assert!(!IpAllowlist::parse(&["0.0.0.0/0"]).unwrap().allows(ip("2001:db8::1")));
        assert!(IpAllowlist::parse(&["10.0.0.0/33"]).is_err());
        assert!(IpAllowlist::parse(&["not-an-ip"]).is_err());
        assert!(IpAllowlist::parse(&["::/129"]).is_err());
    }
}
