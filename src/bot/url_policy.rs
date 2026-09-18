use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::{Host, Url};

#[derive(Debug)]
pub(crate) struct ResolvedDownloadUrl {
    pub url: Url,
    pub host: String,
    pub address: SocketAddr,
}

pub(crate) fn is_unsafe_remote_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_unsafe_ipv4(ip),
        IpAddr::V6(ip) => is_unsafe_ipv6(ip),
    }
}

fn is_unsafe_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    a == 0
        || ip.is_private()
        || (a == 100 && b & 0b1100_0000 == 0b0100_0000)
        || ip.is_loopback()
        || ip.is_link_local()
        || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10)
        || ip.is_documentation()
        || (a == 198 && b & 0xfe == 18)
        || a >= 224
}

fn is_unsafe_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(ipv4) = ip.to_ipv4() {
        return is_unsafe_ipv4(ipv4);
    }
    let segments = ip.segments();
    // Well-Known Prefix NAT64 RFC 6052 64:ff9b::/96
    if segments[0] == 0x64
        && segments[1] == 0xff9b
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0
    {
        let octets = ip.octets();
        let ipv4 = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
        if is_unsafe_ipv4(ipv4) {
            return true;
        }
    }
    // Stateless IP/ICMP Translation (SIIT) RFC 7915 / RFC 2765 ::ffff:0:a.b.c.d/96
    if segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0xffff
        && segments[5] == 0
    {
        let octets = ip.octets();
        let ipv4 = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
        if is_unsafe_ipv4(ipv4) {
            return true;
        }
    }
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || segments[0] & 0xffc0 == 0xfec0
        || matches!(segments, [0x64, 0xff9b, 1, _, _, _, _, _])
        || matches!(segments, [0x100, 0, 0, 0, _, _, _, _])
        || (segments[0] == 0x2001 && segments[1] < 0x200)
        || segments[0] == 0x2002
        || (segments[0] == 0x2001 && segments[1] == 0xdb8)
        || (segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
        || segments[0] == 0x5f00
}

fn parse_download_url(raw: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw.trim()).map_err(|_| "invalid external media URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("external media URL must use http or https with a host".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("external media URL must not contain embedded credentials".to_string());
    }
    if let Some(port) = parsed.port() {
        if !matches!(port, 80 | 443 | 8080 | 8443) {
            return Err("external media URL uses an unsupported port".to_string());
        }
    }
    if let Some(host) = parsed.host() {
        let literal_ip = match host {
            Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
            Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
            Host::Domain(_) => None,
        };
        if literal_ip.is_some_and(is_unsafe_remote_ip) {
            return Err("external media URL points to a blocked network address".to_string());
        }
    }
    Ok(parsed)
}

pub(crate) async fn resolve_download_url(raw: &str) -> Result<ResolvedDownloadUrl, String> {
    let url = parse_download_url(raw)?;
    let host = url
        .host_str()
        .ok_or_else(|| "external media URL has no host".to_string())?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "external media URL has no usable port".to_string())?;
    if !matches!(port, 80 | 443 | 8080 | 8443) {
        return Err("external media URL uses an unsupported port".to_string());
    }

    let resolved = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| "external media host could not be resolved".to_string())?
        .collect::<Vec<_>>();

    if resolved.is_empty() || resolved.iter().any(|addr| is_unsafe_remote_ip(addr.ip())) {
        return Err("external media URL resolved to a blocked network address".to_string());
    }

    Ok(ResolvedDownloadUrl {
        url,
        host,
        address: resolved[0],
    })
}

#[allow(dead_code)]
pub(crate) async fn resolve_redirect_hop(
    current_url: &Url,
    location: &str,
) -> Result<ResolvedDownloadUrl, String> {
    let next_url = current_url
        .join(location.trim())
        .map_err(|e| format!("invalid redirect location: {e}"))?;
    resolve_download_url(next_url.as_str()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_schemes_credentials_and_literal_private_ips() {
        assert!(parse_download_url("file:///etc/passwd").is_err());
        assert!(parse_download_url("ftp://example.com/file").is_err());
        assert!(parse_download_url("https://user:pass@example.com/file").is_err());
        assert!(parse_download_url("http://127.0.0.1/file").is_err());
        assert!(parse_download_url("http://10.0.0.1/file").is_err());
        assert!(parse_download_url("http://169.254.1.1/file").is_err());
        assert!(parse_download_url("http://[::1]/file").is_err());
        assert!(parse_download_url("http://example.com:22/file").is_err());
        assert!(parse_download_url("http://example.com:6379/file").is_err());
        assert!(parse_download_url("http://example.com:8080/file").is_ok());
    }

    #[test]
    fn accepts_public_http_and_https_targets_before_dns_resolution() {
        assert!(parse_download_url("https://example.com/file").is_ok());
        assert!(parse_download_url("http://1.1.1.1/file").is_ok());
    }

    #[test]
    fn remote_ip_policy_matches_private_and_public_boundaries() {
        assert!(is_unsafe_remote_ip("0.0.0.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("100.64.0.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("192.168.1.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("198.18.0.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("240.0.0.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("fc00::1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("2001:db8::1".parse().unwrap()));
        assert!(!is_unsafe_remote_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_unsafe_remote_ip(
            "2606:4700:4700::1111".parse().unwrap()
        ));
    }

    #[test]
    fn blocks_nat64_and_siit_translated_private_ips() {
        assert!(is_unsafe_remote_ip("64:ff9b::127.0.0.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("64:ff9b::192.168.1.1".parse().unwrap()));
        assert!(is_unsafe_remote_ip("::ffff:0:10.0.0.1".parse().unwrap()));
        assert!(!is_unsafe_remote_ip("64:ff9b::1.1.1.1".parse().unwrap()));
        assert!(!is_unsafe_remote_ip("::ffff:0:1.1.1.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_redirect_chain_to_internal_metadata_endpoint() {
        let base = Url::parse("https://public.example.com/media/photo.jpg").unwrap();

        // 1. IP metadata endpoint (169.254.169.254, link-local, fd00::)
        let metadata_targets = [
            "http://169.254.169.254/latest/meta-data",
            "http://169.254.1.1/link-local",
            "http://[fd00::1]/metadata",
            "http://[fe80::1]/link-local",
            "//169.254.169.254/latest/meta-data",
        ];
        for target in metadata_targets {
            let hop = resolve_redirect_hop(&base, target).await;
            assert!(
                hop.is_err(),
                "redirect to metadata target '{target}' must be rejected, got: {hop:?}"
            );
        }

        // 2. Loopback (127.0.0.1, ::1, localhost)
        let loopback_targets = [
            "http://127.0.0.1/admin",
            "http://127.0.0.1:8080/admin",
            "http://[::1]/admin",
            "http://[::1]:8080/admin",
            "http://localhost:8080/admin",
            "//127.0.0.1/admin",
            "//[::1]/admin",
        ];
        for target in loopback_targets {
            let hop = resolve_redirect_hop(&base, target).await;
            assert!(
                hop.is_err(),
                "redirect to loopback target '{target}' must be rejected, got: {hop:?}"
            );
        }

        // 3. Private RFC1918 ranges (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
        let private_targets = [
            "http://10.0.0.1/internal",
            "http://10.255.255.254/internal",
            "http://172.16.0.1/internal",
            "http://172.31.255.254/internal",
            "http://192.168.0.1/internal",
            "http://192.168.1.100/internal",
            "//10.0.0.1/internal",
            "//192.168.1.1/internal",
        ];
        for target in private_targets {
            let hop = resolve_redirect_hop(&base, target).await;
            assert!(
                hop.is_err(),
                "redirect to private target '{target}' must be rejected, got: {hop:?}"
            );
        }

        // 4. Non-HTTP schemes (file://, gopher://, ftp://, javascript:, data:)
        let non_http_targets = [
            "file:///etc/passwd",
            "gopher://127.0.0.1:6379/_",
            "ftp://example.com/file",
            "javascript:alert(1)",
            "data:text/plain;base64,SGVsbG8=",
        ];
        for target in non_http_targets {
            let hop = resolve_redirect_hop(&base, target).await;
            assert!(
                hop.is_err(),
                "redirect to non-http target '{target}' must be rejected, got: {hop:?}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_multi_hop_redirect_chain_terminating_in_private_target() {
        let hop0 = Url::parse("https://public.example.com/start").unwrap();

        // Hop 1: relative redirect to another path on the same public host
        let hop1_url = hop0.join("/intermediate/redirect").unwrap();
        assert_eq!(
            hop1_url.as_str(),
            "https://public.example.com/intermediate/redirect"
        );

        // Hop 2: redirect to AWS/GCP metadata service
        let hop2_meta =
            resolve_redirect_hop(&hop1_url, "http://169.254.169.254/latest/meta-data").await;
        assert!(hop2_meta.is_err());

        // Hop 2: redirect to local loopback
        let hop2_loopback = resolve_redirect_hop(&hop1_url, "http://127.0.0.1:80/secret").await;
        assert!(hop2_loopback.is_err());

        // Hop 2: redirect to RFC1918 private network
        let hop2_rfc1918 = resolve_redirect_hop(&hop1_url, "http://192.168.1.1/admin").await;
        assert!(hop2_rfc1918.is_err());

        // Hop 2: redirect to local filesystem file://
        let hop2_file = resolve_redirect_hop(&hop1_url, "file:///etc/shadow").await;
        assert!(hop2_file.is_err());
    }
}
