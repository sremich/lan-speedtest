//! Working out who a client is.
//!
//! Three separate jobs, deliberately kept apart because they have different
//! trust properties:
//!
//! * **Which address to attribute a run to.** The connection's peer address,
//!   unless the peer is a proxy we were explicitly told to trust — in which
//!   case the leftmost address in `X-Forwarded-For` that is not itself a
//!   trusted proxy. Trusting that header unconditionally would let anyone on
//!   the LAN attribute a run to any address they liked.
//!
//! * **What kind of address it is.** A `10.x` address that arrived through a
//!   Tailscale subnet router looks exactly like a LAN client, because the
//!   router rewrote the source before the packet ever reached us. Saying
//!   "LAN" or "CGNAT" next to the number is the honest amount of certainty
//!   available.
//!
//! * **What it is called.** A reverse lookup, restricted to the address
//!   ranges an operator configures. Unrestricted, a public-facing deployment
//!   would start sending PTR queries for internet addresses to an upstream
//!   resolver, which is a quiet leak this project should not have.
//!
//! The DNS client here is deliberately minimal — one query, one datagram, no
//! retries — rather than a resolver crate. It is used for a cosmetic label on
//! a LAN tool, and the whole surface is about a hundred lines that can be read
//! in one sitting.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

/// A parsed CIDR block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let (addr_part, prefix_part) = s.split_once('/').ok_or_else(|| {
            format!("{s:?} is not a CIDR block — expected something like 10.0.0.0/8")
        })?;

        let addr: IpAddr = addr_part
            .trim()
            .parse()
            .map_err(|_| format!("{addr_part:?} is not an IP address"))?;
        let prefix: u8 = prefix_part
            .trim()
            .parse()
            .map_err(|_| format!("{prefix_part:?} is not a prefix length"))?;

        let max = if addr.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return Err(format!("/{prefix} is too long for {addr}"));
        }
        Ok(Self { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let (network, candidate) = match (self.addr, normalise(ip)) {
            (IpAddr::V4(a), IpAddr::V4(b)) => (a.octets().to_vec(), b.octets().to_vec()),
            (IpAddr::V6(a), IpAddr::V6(b)) => (a.octets().to_vec(), b.octets().to_vec()),
            // Different families never match; comparing them would be a bug
            // that reads as a working allow-list.
            _ => return false,
        };

        let whole = (self.prefix / 8) as usize;
        if network[..whole] != candidate[..whole] {
            return false;
        }
        let remainder = self.prefix % 8;
        if remainder == 0 {
            return true;
        }
        let mask = 0xFFu8 << (8 - remainder);
        network[whole] & mask == candidate[whole] & mask
    }
}

/// Unwraps `::ffff:a.b.c.d` so a v4 client behind a v6 socket is matched by v4
/// rules — otherwise every private-range check silently misses.
pub fn normalise(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

/// What kind of address this is, to the extent that can be known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Loopback,
    LinkLocal,
    /// 100.64.0.0/10. Tailscale's range, and also carrier-grade NAT.
    Cgnat,
    Private,
    Public,
}

impl Kind {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::LinkLocal => "link-local",
            Self::Cgnat => "cgnat",
            Self::Private => "lan",
            Self::Public => "public",
        }
    }

    /// Shown next to the address. Hedged where hedging is honest: a private
    /// address may be the client or may be a router that rewrote it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::LinkLocal => "link-local",
            Self::Cgnat => "Tailscale or carrier NAT",
            Self::Private => "LAN",
            Self::Public => "public",
        }
    }
}

const CGNAT: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 0);

pub fn classify(ip: IpAddr) -> Kind {
    match normalise(ip) {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                Kind::Loopback
            } else if v4.is_link_local() {
                Kind::LinkLocal
            } else if in_cgnat(v4) {
                Kind::Cgnat
            } else if v4.is_private() {
                Kind::Private
            } else {
                Kind::Public
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                Kind::Loopback
            } else if is_unicast_link_local(v6) {
                Kind::LinkLocal
            } else if is_unique_local(v6) {
                Kind::Private
            } else {
                Kind::Public
            }
        }
    }
}

fn in_cgnat(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    let n = CGNAT.octets();
    o[0] == n[0] && (o[1] & 0xC0) == n[1]
}

/// `fe80::/10`. `Ipv6Addr::is_unicast_link_local` is still unstable.
fn is_unicast_link_local(v6: Ipv6Addr) -> bool {
    v6.segments()[0] & 0xffc0 == 0xfe80
}

/// `fc00::/7`. `Ipv6Addr::is_unique_local` is still unstable.
fn is_unique_local(v6: Ipv6Addr) -> bool {
    v6.segments()[0] & 0xfe00 == 0xfc00
}

/// The address to attribute a run to.
///
/// `forwarded_for` is the raw `X-Forwarded-For` header, if present. It is
/// consulted *only* when the peer is one of the configured trusted proxies;
/// otherwise it is ignored entirely, however plausible it looks.
pub fn effective_client(peer: IpAddr, forwarded_for: Option<&str>, trusted: &[Cidr]) -> IpAddr {
    if trusted.is_empty() || !trusted.iter().any(|c| c.contains(peer)) {
        return normalise(peer);
    }

    let Some(header) = forwarded_for else {
        return normalise(peer);
    };

    // Right to left: each hop appends, so the rightmost entries are the ones
    // our own trusted proxies added. Walk back through those and stop at the
    // first address no trusted proxy vouched for — anything further left was
    // written by the client and is not evidence of anything.
    let mut candidate = normalise(peer);
    for entry in header.split(',').rev() {
        let Ok(ip) = entry.trim().parse::<IpAddr>() else {
            break;
        };
        let ip = normalise(ip);
        candidate = ip;
        if !trusted.iter().any(|c| c.contains(ip)) {
            break;
        }
    }
    candidate
}

/* --- reverse DNS ---------------------------------------------------------- */

/// The `in-addr.arpa` / `ip6.arpa` name whose PTR record names this address.
pub fn reverse_name(ip: IpAddr) -> String {
    match normalise(ip) {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
        }
        IpAddr::V6(v6) => {
            let mut out = String::with_capacity(72);
            for byte in v6.octets().iter().rev() {
                out.push_str(&format!("{:x}.{:x}.", byte & 0x0f, byte >> 4));
            }
            out.push_str("ip6.arpa");
            out
        }
    }
}

/// Query ids. Not cryptographic randomness, and deliberately not pretending to
/// be: the reply is also checked against the question we asked, and this talks
/// to a resolver on the same LAN.
static QUERY_ID: AtomicU16 = AtomicU16::new(0);

fn next_id() -> u16 {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u16)
        .unwrap_or(1);
    QUERY_ID
        .fetch_add(seed | 1, Ordering::Relaxed)
        .wrapping_add(seed)
}

/// Encodes a PTR query for `name`.
pub fn encode_query(id: u16, name: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(name.len() + 18);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, recursion desired
    out.extend_from_slice(&1u16.to_be_bytes()); // one question
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // no answer/authority/additional

    for label in name.split('.') {
        // Labels are capped at 63 bytes by the protocol; ours are generated,
        // never user input, so a longer one is a bug rather than an attack.
        let bytes = label.as_bytes();
        out.push(bytes.len().min(63) as u8);
        out.extend_from_slice(&bytes[..bytes.len().min(63)]);
    }
    out.push(0);
    out.extend_from_slice(&12u16.to_be_bytes()); // PTR
    out.extend_from_slice(&1u16.to_be_bytes()); // IN
    out
}

/// Reads a possibly-compressed domain name, returning it and the offset just
/// past it in the *stream* (not past the pointer target).
///
/// Every step is bounds-checked and pointer following is capped, because a
/// malicious or broken response must not be able to hang the server. A pointer
/// that refers to itself is the classic way to do that.
fn read_name(msg: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = start;
    let mut jumps = 0usize;
    let mut resume: Option<usize> = None;
    let mut total = 0usize;

    loop {
        let len = *msg.get(pos)?;

        if len & 0xC0 == 0xC0 {
            let low = *msg.get(pos + 1)? as usize;
            let target = (((len & 0x3F) as usize) << 8) | low;
            resume.get_or_insert(pos + 2);
            jumps += 1;
            if jumps > 16 || target >= msg.len() {
                return None;
            }
            pos = target;
            continue;
        }

        if len & 0xC0 != 0 {
            return None; // reserved label type
        }

        if len == 0 {
            pos += 1;
            break;
        }

        let from = pos + 1;
        let to = from.checked_add(len as usize)?;
        let label = msg.get(from..to)?;
        total += label.len() + 1;
        if total > 255 {
            return None;
        }
        labels.push(String::from_utf8(label.to_vec()).ok()?);
        pos = to;
    }

    Some((labels.join("."), resume.unwrap_or(pos)))
}

/// Skips a name without building it, for the parts we do not need.
fn skip_name(msg: &[u8], start: usize) -> Option<usize> {
    read_name(msg, start).map(|(_, next)| next)
}

/// A hostname we are willing to store and display.
///
/// A PTR record can contain any bytes at all. Restricting to the characters a
/// hostname may actually hold means nothing surprising reaches the database or
/// the page, regardless of what answered.
pub fn acceptable_hostname(name: &str) -> Option<String> {
    let trimmed = name.trim_end_matches('.').trim();
    if trimmed.is_empty() || trimmed.len() > 253 {
        return None;
    }
    let ok = trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    ok.then(|| trimmed.to_ascii_lowercase())
}

/// Pulls the first PTR name out of a response, or `None` if there is not one
/// we can trust.
pub fn parse_ptr_response(msg: &[u8], expect_id: u16, expect_name: &str) -> Option<String> {
    if msg.len() < 12 {
        return None;
    }
    let id = u16::from_be_bytes([msg[0], msg[1]]);
    let flags = u16::from_be_bytes([msg[2], msg[3]]);
    if id != expect_id || flags & 0x8000 == 0 || flags & 0x000F != 0 {
        return None;
    }

    let questions = u16::from_be_bytes([msg[4], msg[5]]);
    let answers = u16::from_be_bytes([msg[6], msg[7]]);
    if questions != 1 || answers == 0 {
        return None;
    }

    // The question must be the one we asked. Combined with the id, this is
    // what makes an off-path forgery need to guess both.
    let (asked, mut pos) = read_name(msg, 12)?;
    if !asked.eq_ignore_ascii_case(expect_name.trim_end_matches('.')) {
        return None;
    }
    pos += 4; // qtype + qclass

    for _ in 0..answers {
        pos = skip_name(msg, pos)?;
        let rtype = u16::from_be_bytes([*msg.get(pos)?, *msg.get(pos + 1)?]);
        let rdlen = u16::from_be_bytes([*msg.get(pos + 8)?, *msg.get(pos + 9)?]) as usize;
        let rdata = pos + 10;
        if rdata.checked_add(rdlen)? > msg.len() {
            return None;
        }
        if rtype == 12 {
            let (name, _) = read_name(msg, rdata)?;
            return acceptable_hostname(&name);
        }
        pos = rdata + rdlen;
    }
    None
}

/// The first `nameserver` in `/etc/resolv.conf`.
///
/// Read at lookup time rather than at startup: in a container the file can be
/// rewritten under us, and a stale resolver would fail silently.
pub fn resolver_from_resolv_conf(contents: &str) -> Option<SocketAddr> {
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("nameserver") else {
            continue;
        };
        let Ok(ip) = rest.trim().parse::<IpAddr>() else {
            continue;
        };
        return Some(SocketAddr::new(ip, 53));
    }
    None
}

/// Asks `resolver` what `ip` is called.
///
/// One datagram, one wait, no retries: this is a label on a history row, and a
/// resolver that did not answer promptly has answered.
pub async fn reverse_lookup(resolver: SocketAddr, ip: IpAddr, timeout: Duration) -> Option<String> {
    let name = reverse_name(ip);
    let id = next_id();
    let query = encode_query(id, &name);

    let bind: SocketAddr = if resolver.is_ipv4() {
        "0.0.0.0:0".parse().ok()?
    } else {
        "[::]:0".parse().ok()?
    };

    let socket = tokio::net::UdpSocket::bind(bind).await.ok()?;
    socket.connect(resolver).await.ok()?;
    socket.send(&query).await.ok()?;

    let mut buf = [0u8; 1232];
    let read = tokio::time::timeout(timeout, socket.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    parse_ptr_response(&buf[..read], id, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn cidr_matches_only_inside_its_block() {
        let block = Cidr::parse("10.42.7.0/24").unwrap();
        assert!(block.contains(ip("10.42.7.1")));
        assert!(block.contains(ip("10.42.7.255")));
        assert!(!block.contains(ip("10.42.8.1")));
        assert!(!block.contains(ip("10.0.0.1")));

        // A non-byte-aligned prefix is where an off-by-one hides.
        let half = Cidr::parse("10.0.0.0/9").unwrap();
        assert!(half.contains(ip("10.127.255.255")));
        assert!(!half.contains(ip("10.128.0.0")));
    }

    #[test]
    fn a_v4_mapped_v6_address_is_matched_by_v4_rules() {
        // A v4 client on a dual-stack socket arrives as ::ffff:10.0.0.5. Left
        // unnormalised, every private-range check silently misses and the
        // address reads as public.
        let lan = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(lan.contains(ip("::ffff:10.0.0.5")));
        assert_eq!(classify(ip("::ffff:10.0.0.5")), Kind::Private);
    }

    #[test]
    fn a_cidr_never_matches_across_families() {
        let v4 = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(!v4.contains(ip("2001:db8::1")));
    }

    #[test]
    fn addresses_are_classified_by_what_they_actually_are() {
        assert_eq!(classify(ip("127.0.0.1")), Kind::Loopback);
        assert_eq!(classify(ip("10.42.7.3")), Kind::Private);
        assert_eq!(classify(ip("192.168.1.9")), Kind::Private);
        assert_eq!(classify(ip("169.254.4.4")), Kind::LinkLocal);
        assert_eq!(classify(ip("8.8.8.8")), Kind::Public);
        assert_eq!(classify(ip("fd00::1")), Kind::Private);
        assert_eq!(classify(ip("fe80::1")), Kind::LinkLocal);
        assert_eq!(classify(ip("2001:db8::1")), Kind::Public);
    }

    #[test]
    fn the_tailscale_range_is_called_out_rather_than_read_as_the_internet() {
        // 100.64.0.0/10 is Tailscale's, and also carrier NAT. Either way it is
        // not a LAN address and not a public one.
        assert_eq!(classify(ip("100.64.0.1")), Kind::Cgnat);
        assert_eq!(classify(ip("100.127.255.254")), Kind::Cgnat);
        assert_eq!(classify(ip("100.128.0.1")), Kind::Public);
        assert_eq!(classify(ip("100.63.255.255")), Kind::Public);
    }

    #[test]
    fn forwarded_for_is_ignored_unless_the_peer_is_a_trusted_proxy() {
        // The whole point. Without this, anyone on the LAN can attribute a run
        // to any address they like by setting one header.
        let trusted = vec![Cidr::parse("10.0.0.1/32").unwrap()];
        assert_eq!(
            effective_client(ip("10.0.0.9"), Some("1.2.3.4"), &trusted),
            ip("10.0.0.9"),
            "an untrusted peer's header must not be believed"
        );
        assert_eq!(
            effective_client(ip("10.0.0.1"), Some("1.2.3.4"), &trusted),
            ip("1.2.3.4"),
            "a trusted proxy's header should be believed"
        );
    }

    #[test]
    fn a_chain_of_trusted_proxies_resolves_to_the_first_untrusted_hop() {
        let trusted = vec![
            Cidr::parse("10.0.0.0/24").unwrap(),
            Cidr::parse("192.168.5.0/24").unwrap(),
        ];
        // client -> 192.168.5.2 -> 10.0.0.1 -> us
        assert_eq!(
            effective_client(ip("10.0.0.1"), Some("203.0.113.7, 192.168.5.2"), &trusted,),
            ip("203.0.113.7"),
        );
    }

    #[test]
    fn a_forged_prefix_cannot_reach_past_the_trusted_hops() {
        // The client wrote "9.9.9.9" itself before any proxy saw it; only the
        // rightmost entries were added by hops we trust.
        let trusted = vec![Cidr::parse("10.0.0.0/24").unwrap()];
        assert_eq!(
            effective_client(ip("10.0.0.1"), Some("9.9.9.9, 203.0.113.7"), &trusted),
            ip("203.0.113.7"),
        );
    }

    #[test]
    fn no_trusted_proxies_means_the_header_is_never_read() {
        assert_eq!(
            effective_client(ip("10.0.0.1"), Some("1.2.3.4"), &[]),
            ip("10.0.0.1")
        );
    }

    #[test]
    fn reverse_names_follow_the_arpa_conventions() {
        assert_eq!(
            reverse_name(ip("10.42.7.50")),
            "50.7.42.10.in-addr.arpa"
        );
        assert!(reverse_name(ip("2001:db8::1")).ends_with(".ip6.arpa"));
        // 32 nibbles, each with its dot, plus the suffix.
        assert_eq!(
            reverse_name(ip("2001:db8::1")).len(),
            32 * 2 + "ip6.arpa".len()
        );
    }

    /// Builds a response for `question` answering with `answer`.
    fn response(id: u16, question: &str, answer: &str, compress: bool) -> Vec<u8> {
        let mut msg = encode_query(id, question);
        msg[2] = 0x81; // QR + RD
        msg[3] = 0x80; // RA
        msg[6..8].copy_from_slice(&1u16.to_be_bytes()); // one answer

        // Answer: a pointer back to the question name, or the name in full.
        if compress {
            msg.extend_from_slice(&[0xC0, 12]);
        } else {
            for label in question.split('.') {
                msg.push(label.len() as u8);
                msg.extend_from_slice(label.as_bytes());
            }
            msg.push(0);
        }
        msg.extend_from_slice(&12u16.to_be_bytes()); // PTR
        msg.extend_from_slice(&1u16.to_be_bytes()); // IN
        msg.extend_from_slice(&300u32.to_be_bytes()); // TTL

        let mut rdata = Vec::new();
        for label in answer.split('.') {
            rdata.push(label.len() as u8);
            rdata.extend_from_slice(label.as_bytes());
        }
        rdata.push(0);
        msg.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        msg.extend_from_slice(&rdata);
        msg
    }

    #[test]
    fn a_ptr_answer_is_read_back() {
        let q = reverse_name(ip("10.42.7.50"));
        let msg = response(0x1234, &q, "speed.example.internal", false);
        assert_eq!(
            parse_ptr_response(&msg, 0x1234, &q).as_deref(),
            Some("speed.example.internal")
        );
    }

    #[test]
    fn a_compressed_answer_name_is_followed() {
        // Real resolvers compress. A parser that cannot follow a pointer would
        // work against a hand-built fixture and fail against every real one.
        let q = reverse_name(ip("10.42.7.50"));
        let msg = response(0x1234, &q, "host.example", true);
        // The answer's *owner* name is the compressed part here; the record
        // still parses and yields its rdata.
        assert_eq!(
            parse_ptr_response(&msg, 0x1234, &q).as_deref(),
            Some("host.example")
        );
    }

    #[test]
    fn a_pointer_loop_terminates_instead_of_hanging() {
        // The property that matters most: a malicious or broken response must
        // not be able to spin the server. This message points at itself.
        let q = reverse_name(ip("10.0.0.1"));
        let mut msg = encode_query(0x1234, &q);
        msg[2] = 0x81;
        msg[3] = 0x80;
        msg[6..8].copy_from_slice(&1u16.to_be_bytes());
        let here = msg.len();
        msg.extend_from_slice(&[0xC0, here as u8]); // points at itself
        msg.extend_from_slice(&12u16.to_be_bytes());
        msg.extend_from_slice(&1u16.to_be_bytes());
        msg.extend_from_slice(&0u32.to_be_bytes());
        msg.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(parse_ptr_response(&msg, 0x1234, &q), None);
    }

    #[test]
    fn a_truncated_message_is_rejected_rather_than_panicking() {
        let q = reverse_name(ip("10.42.7.50"));
        let full = response(0x1234, &q, "host.example", false);
        for cut in 0..full.len() {
            // Must not panic at any truncation point.
            let _ = parse_ptr_response(&full[..cut], 0x1234, &q);
        }
    }

    #[test]
    fn a_response_to_a_different_question_is_refused() {
        // Together with the id, this is what an off-path forgery has to guess.
        let asked = reverse_name(ip("10.42.7.50"));
        let other = reverse_name(ip("10.42.7.51"));
        let msg = response(0x1234, &other, "wrong.example", false);
        assert_eq!(parse_ptr_response(&msg, 0x1234, &asked), None);
        assert_eq!(parse_ptr_response(&msg, 0x9999, &other), None);
    }

    #[test]
    fn an_error_response_yields_nothing() {
        let q = reverse_name(ip("10.0.0.1"));
        let mut msg = response(0x1234, &q, "host.example", false);
        msg[3] |= 0x03; // NXDOMAIN
        assert_eq!(parse_ptr_response(&msg, 0x1234, &q), None);
    }

    #[test]
    fn only_plausible_hostnames_are_accepted() {
        assert_eq!(
            acceptable_hostname("Host.Example."),
            Some("host.example".into())
        );
        assert_eq!(acceptable_hostname("nas-01.lan"), Some("nas-01.lan".into()));
        assert_eq!(acceptable_hostname(""), None);
        assert_eq!(acceptable_hostname("."), None);
        // A PTR record can hold arbitrary bytes; none of them should reach the
        // database or the page.
        assert_eq!(acceptable_hostname("<script>alert(1)</script>"), None);
        assert_eq!(acceptable_hostname("has space"), None);
        assert_eq!(acceptable_hostname(&"a".repeat(300)), None);
    }

    #[test]
    fn the_resolver_is_read_from_resolv_conf() {
        let conf = "# comment\nsearch lan\nnameserver 10.42.7.1\nnameserver 1.1.1.1\n";
        assert_eq!(
            resolver_from_resolv_conf(conf),
            Some("10.42.7.1:53".parse().unwrap())
        );
        assert_eq!(resolver_from_resolv_conf("search lan\n"), None);
        assert_eq!(resolver_from_resolv_conf("nameserver nonsense\n"), None);
    }
}
