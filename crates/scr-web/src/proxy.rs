//! Explicit proxy trust and strict request authority validation.
use hyper::HeaderMap;
use std::{
    io,
    net::{IpAddr, SocketAddr},
};

const MAX_ENTRIES: usize = 128;
const MAX_HOPS: usize = 32;

struct Network {
    address: IpAddr,
    prefix: u8,
}
impl Network {
    fn parse(value: &str) -> Option<Self> {
        let (address, prefix) = value
            .split_once('/')
            .map_or((value, None), |(a, p)| (a, Some(p)));
        let address: IpAddr = address.parse().ok()?;
        let bits = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok()?,
            Some(_) => return None,
            None => bits,
        };
        (prefix <= bits).then_some(Self { address, prefix })
    }
    fn contains(&self, address: IpAddr) -> bool {
        // IPv4-mapped IPv6 is deliberately a separate family; trust must be explicit.
        match (self.address, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = u32::MAX
                    .checked_shl(u32::from(32 - self.prefix))
                    .unwrap_or(0);
                u32::from(network) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = u128::MAX
                    .checked_shl(u32::from(128 - self.prefix))
                    .unwrap_or(0);
                u128::from(network) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq)]
struct Authority {
    hostname: String,
    port: Option<u16>,
}
impl Authority {
    fn parse(value: &str) -> Option<Self> {
        if value.is_empty() || value.len() > 260 || !value.is_ascii() {
            return None;
        }
        let (hostname, port) = if let Some(rest) = value.strip_prefix('[') {
            let (ip, tail) = rest.split_once(']')?;
            let ip: std::net::Ipv6Addr = ip.parse().ok()?;
            let port = if tail.is_empty() {
                None
            } else {
                Some(tail.strip_prefix(':')?)
            };
            (format!("[{ip}]"), port)
        } else {
            let (host, port) = value
                .split_once(':')
                .map_or((value, None), |(h, p)| (h, Some(p)));
            // ASCII DNS labels, including localhost, or an IPv4 literal. One final
            // DNS root dot is normalized away. No userinfo, escapes, or wildcards.
            let host = host.strip_suffix('.').unwrap_or(host);
            if host.is_empty()
                || host.len() > 253
                || !host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label.as_bytes()[0].is_ascii_alphanumeric()
                        && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return None;
            }
            (host.to_ascii_lowercase(), port)
        };
        let port = match port {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                Some(p.parse::<u16>().ok().filter(|p| *p != 0)?)
            }
            Some(_) => return None,
            None => None,
        };
        Some(Self { hostname, port })
    }
    fn display(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.hostname),
            None => self.hostname.clone(),
        }
    }
}

pub(crate) struct Policy {
    trusted: Vec<Network>,
    allowed: Vec<Authority>,
}
impl Policy {
    pub fn new(trusted: &[String], allowed: &[String]) -> io::Result<Self> {
        if trusted.len() > MAX_ENTRIES || allowed.len() > MAX_ENTRIES {
            return Err(io::Error::other(
                "trusted-proxies and allowed-hosts permit at most 128 entries each",
            ));
        }
        let trusted = trusted
            .iter()
            .map(|s| {
                (s.len() <= 64)
                    .then(|| Network::parse(s))
                    .flatten()
                    .ok_or_else(|| {
                        io::Error::other(
                            "trusted-proxies requires literal IP addresses or valid CIDRs",
                        )
                    })
            })
            .collect::<io::Result<_>>()?;
        let allowed = allowed.iter().map(|s| Authority::parse(s)
            .ok_or_else(|| io::Error::other("allowed-hosts requires ASCII hostnames or bracketed IPv6 authorities with optional ports")))
            .collect::<io::Result<_>>()?;
        Ok(Self { trusted, allowed })
    }
    fn trusts(&self, ip: IpAddr) -> bool {
        self.trusted.iter().any(|n| n.contains(ip))
    }
    pub fn resolve(
        &self,
        peer: SocketAddr,
        headers: &HeaderMap,
        require_host: bool,
    ) -> Result<scr_stdlib::web::ConnectionInfo, u16> {
        let mut host = single(headers, "host")?
            .map(|h| Authority::parse(h).ok_or(400u16))
            .transpose()?;
        if require_host && host.is_none() {
            return Err(400);
        }
        let mut client = peer.ip();
        let mut scheme = "http";
        if self.trusts(peer.ip()) {
            if let Some(value) = single(headers, "x-forwarded-for")? {
                // Parse the entire bounded list before using it; even discarded
                // prefix entries cannot smuggle malformed header syntax.
                let mut chain = Vec::new();
                for item in value.split(',') {
                    if chain.len() == MAX_HOPS {
                        return Err(400);
                    }
                    chain.push(
                        item.trim_matches([' ', '\t'])
                            .parse::<IpAddr>()
                            .map_err(|_| 400u16)?,
                    );
                }
                for ip in chain.into_iter().rev() {
                    if !self.trusts(client) {
                        break;
                    }
                    client = ip;
                }
            }
            if let Some(value) = single(headers, "x-forwarded-proto")? {
                scheme = match value {
                    "http" => "http",
                    "https" => "https",
                    _ => return Err(400),
                };
            }
            if let Some(value) = single(headers, "x-forwarded-host")? {
                host = Some(Authority::parse(value).ok_or(400u16)?);
            }
        }
        if !self.allowed.is_empty()
            && !host.as_ref().is_some_and(|host| {
                self.allowed.iter().any(|allowed| {
                    host.hostname == allowed.hostname
                        && (allowed.port.is_none() || allowed.port == host.port)
                })
            })
        {
            return Err(400);
        }
        Ok(scr_stdlib::web::ConnectionInfo {
            peer_address: Some(peer),
            client_address: Some(client),
            scheme: Some(scheme.into()),
            host: host.map(|h| h.display()),
        })
    }
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, u16> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|v| v.to_str().map_err(|_| 400u16))
        .transpose()?;
    if values.next().is_some() {
        return Err(400);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy(trusted: &[&str], allowed: &[&str]) -> Policy {
        Policy::new(
            &trusted.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &allowed.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn headers(fields: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in fields {
            headers.append(
                hyper::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }
    #[test]
    fn cidr_boundaries_families_and_invalid_configuration() {
        for (cidr, yes, no) in [
            ("192.0.2.99/24", "192.0.2.255", "192.0.3.0"),
            ("::1/128", "::1", "::2"),
            ("2001:db8::/32", "2001:db8:ffff::1", "2001:db9::"),
            ("0.0.0.0/0", "255.255.255.255", "::ffff:192.0.2.1"),
            ("::/0", "ffff::1", "192.0.2.1"),
        ] {
            let policy = policy(&[cidr], &[]);
            assert!(policy.trusts(yes.parse().unwrap()));
            assert!(!policy.trusts(no.parse().unwrap()));
        }
        for invalid in [
            "localhost",
            "*",
            "127.0.0.1:80",
            "1.2.3.4/33",
            "::1/129",
            "::1/-1",
            "::1/+1",
            "::1/",
            "::1/1/2",
            "",
        ] {
            assert!(Policy::new(&[invalid.into()], &[]).is_err(), "{invalid}");
        }
        assert!(Policy::new(&vec!["127.0.0.1".into(); 129], &[]).is_err());
        assert!(Policy::new(&[], &vec!["example.com".into(); 129]).is_err());
        assert!(Policy::new(&vec!["127.0.0.1".into(); 128], &[]).is_ok());
    }
    #[test]
    fn authority_normalization_ports_and_rejected_spellings() {
        for (input, expected) in [
            ("EXAMPLE.com.:00443", "example.com:443"),
            ("[2001:0db8::1]:80", "[2001:db8::1]:80"),
            ("localhost", "localhost"),
            ("127.0.0.1:65535", "127.0.0.1:65535"),
        ] {
            assert_eq!(Authority::parse(input).unwrap().display(), expected);
        }
        for invalid in [
            "",
            "example.com:",
            "example.com:0",
            "example.com:65536",
            "example.com:+80",
            "example.com:-1",
            "example.com:80:90",
            "user@example.com",
            "example.com/path",
            "example.com?x",
            "example.com#x",
            "example.com,evil",
            "example.com..",
            ".example.com",
            "*.example.com",
            "foo_bar",
            "-bad.com",
            "bad-.com",
            "a b",
            "example%2ecom",
            "é.com",
            "::1",
            "[::1]junk",
            "[::1%lo0]",
            "[127.0.0.1]",
        ] {
            assert!(Authority::parse(invalid).is_none(), "{invalid}");
            assert!(Policy::new(&[], &[invalid.into()]).is_err());
        }
        let maximum = format!(
            "{}.{}.{}.{}.:65535",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        assert_eq!(maximum.len(), 260);
        assert!(Authority::parse(&maximum).is_some());
        assert!(Authority::parse(&format!("{}.com", "a".repeat(64))).is_none());
    }
    #[test]
    fn untrusted_forwarded_fields_including_duplicates_are_not_identity() {
        let headers = headers(&[
            ("host", "LOCALHOST:8080"),
            ("x-forwarded-for", "unknown"),
            ("x-forwarded-for", "192.0.2.1"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "evil.example"),
        ]);
        let peer = "127.0.0.1:12345".parse().unwrap();
        let info = policy(&[], &[]).resolve(peer, &headers, true).unwrap();
        assert_eq!(info.peer_address, Some(peer));
        assert_eq!(info.client_address, Some(peer.ip()));
        assert_eq!(info.scheme.as_deref(), Some("http"));
        assert_eq!(info.host.as_deref(), Some("localhost:8080"));
        // Unsupported header families never override the selected X-Forwarded policy.
        let headers = self::headers(&[
            ("host", "localhost"),
            ("forwarded", "for=192.0.2.1;proto=https;host=evil"),
            ("x-real-ip", "192.0.2.1"),
        ]);
        let info = policy(&["127.0.0.1"], &[])
            .resolve(peer, &headers, true)
            .unwrap();
        assert_eq!(info.client_address, Some(peer.ip()));
        assert_eq!(info.scheme.as_deref(), Some("http"));
    }
    #[test]
    fn trusted_chain_stops_at_nearest_untrusted_hop_and_handles_ipv6() {
        let policy = policy(&["127.0.0.1", "10.0.0.0/8", "2001:db8::/32"], &[]);
        for (chain, expected) in [
            ("198.51.100.9, 192.0.2.1, 10.1.2.3", "192.0.2.1"),
            ("198.51.100.9, 2001:db8::1", "198.51.100.9"),
            ("10.2.3.4", "10.2.3.4"),
        ] {
            let headers = headers(&[
                ("host", "localhost"),
                ("x-forwarded-for", chain),
                ("x-forwarded-proto", "https"),
                ("x-forwarded-host", "EXAMPLE.com:443"),
            ]);
            let info = policy
                .resolve("127.0.0.1:1".parse().unwrap(), &headers, true)
                .unwrap();
            assert_eq!(info.client_address, Some(expected.parse().unwrap()));
            assert_eq!(info.scheme.as_deref(), Some("https"));
            assert_eq!(info.host.as_deref(), Some("example.com:443"));
        }
        let headers = headers(&[("host", "[::1]:8080"), ("x-forwarded-for", "2001:db9::1")]);
        let peer = "[2001:db8::10]:123".parse().unwrap();
        let info = policy.resolve(peer, &headers, true).unwrap();
        assert_eq!(info.peer_address, Some(peer));
        assert_eq!(info.client_address, Some("2001:db9::1".parse().unwrap()));
    }
    #[test]
    fn trusted_malformed_or_duplicate_metadata_fails_closed() {
        let policy = policy(&["127.0.0.1"], &[]);
        let peer = "127.0.0.1:1".parse().unwrap();
        for (key, invalid) in [
            ("x-forwarded-for", ""),
            ("x-forwarded-for", "unknown"),
            ("x-forwarded-for", "192.0.2.1,"),
            ("x-forwarded-for", "192.0.2.1:80"),
            ("x-forwarded-for", "[::1]"),
            ("x-forwarded-proto", "https,http"),
            ("x-forwarded-proto", "ftp"),
            ("x-forwarded-host", "example.com,evil.com"),
        ] {
            assert_eq!(
                policy
                    .resolve(
                        peer,
                        &headers(&[("host", "localhost"), (key, invalid)]),
                        true
                    )
                    .unwrap_err(),
                400
            );
        }
        for key in [
            "host",
            "x-forwarded-for",
            "x-forwarded-proto",
            "x-forwarded-host",
        ] {
            let value = match key {
                "x-forwarded-for" => "192.0.2.1",
                "x-forwarded-proto" => "https",
                _ => "localhost",
            };
            let mut h = headers(&[("host", "localhost")]);
            h.append(
                hyper::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            h.append(
                hyper::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            assert_eq!(policy.resolve(peer, &h, true).unwrap_err(), 400);
        }
        for hops in [32, 33] {
            let chain = vec!["192.0.2.1"; hops].join(",");
            assert_eq!(
                policy
                    .resolve(
                        peer,
                        &headers(&[("host", "localhost"), ("x-forwarded-for", &chain)]),
                        true
                    )
                    .is_ok(),
                hops == 32
            );
        }
    }
    #[test]
    fn allowlist_checks_effective_host_and_explicit_ports_without_suffix_matching() {
        let policy = policy(&["127.0.0.1"], &["EXAMPLE.com.", "[::1]:443"]);
        let peer = "127.0.0.1:1".parse().unwrap();
        for (host, accepted) in [
            ("example.com", true),
            ("EXAMPLE.com.:8080", true),
            ("sub.example.com", false),
            ("example.com.evil", false),
            ("[::1]:443", true),
            ("[::1]", false),
            ("[::1]:80", false),
        ] {
            assert_eq!(
                policy
                    .resolve(peer, &headers(&[("host", host)]), true)
                    .is_ok(),
                accepted,
                "{host}"
            );
        }
        assert!(
            policy
                .resolve(
                    peer,
                    &headers(&[("host", "internal"), ("x-forwarded-host", "example.com")]),
                    true
                )
                .is_ok()
        );
        assert!(
            policy
                .resolve(
                    peer,
                    &headers(&[("host", "example.com"), ("x-forwarded-host", "evil.com")]),
                    true
                )
                .is_err()
        );
        assert!(policy.resolve(peer, &HeaderMap::new(), false).is_err());
        assert!(
            self::policy(&[], &[])
                .resolve(peer, &HeaderMap::new(), false)
                .unwrap()
                .host
                .is_none()
        );
        assert!(
            self::policy(&[], &[])
                .resolve(peer, &HeaderMap::new(), true)
                .is_err()
        );
    }
}
