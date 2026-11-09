use std::error::Error;
use axum_server::tls_rustls::RustlsConfig;

/// Discovers active local IP addresses by querying the kernel routing table via UDP sockets.
/// UDP socket connect in POSIX/Linux/Windows only triggers a routing table lookup; no packets are sent.
pub fn discover_local_ips() -> Vec<std::net::IpAddr> {
    let mut ips = Vec::new();
    let probe_targets = [
        "8.8.8.8:80",
        "1.1.1.1:80",
        "10.255.255.255:80",  // Class A private network
        "172.16.0.1:80",      // Class B private network
        "192.168.1.1:80",     // Class C private network
        "192.168.0.1:80",
    ];
    for target in probe_targets {
        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if socket.connect(target).is_ok() {
                if let Ok(local_addr) = socket.local_addr() {
                    let ip = local_addr.ip();
                    // Exclude loopback, unspecified, and CGNAT/VPN (100.64.0.0/10) to protect private network IPs
                    let is_cgnat_or_vpn = match ip {
                        std::net::IpAddr::V4(ipv4) => ipv4.octets()[0] == 100,
                        _ => false,
                    };
                    if !ip.is_loopback() && !ip.is_unspecified() && !is_cgnat_or_vpn && !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
            }
        }
    }
    ips
}

/// Generates a self-signed X.509 certificate and private key in PEM format using rcgen,
/// including localhost, 127.0.0.1, auto-discovered local network IPs (LAN/Wi-Fi), and any extra IPs.
pub fn generate_self_signed_cert_pem_with_ips(extra_ips: &[std::net::IpAddr]) -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error + Send + Sync>> {
    let mut params = rcgen::CertificateParams::default();
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "WatchCam Surveillance Node");
    dn.push(rcgen::DnType::OrganizationName, "WatchCam Security");
    params.distinguished_name = dn;

    let mut sans = vec![
        rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
    ];
    if let Ok(dns) = rcgen::Ia5String::try_from("localhost".to_string()) {
        sans.push(rcgen::SanType::DnsName(dns));
    }

    // Auto-discover local active IPs (LAN/Wi-Fi)
    for ip in discover_local_ips() {
        if !sans.iter().any(|s| match s {
            rcgen::SanType::IpAddress(existing) => existing == &ip,
            _ => false,
        }) {
            sans.push(rcgen::SanType::IpAddress(ip));
        }
    }

    // Append any explicitly provided IPs
    for &ip in extra_ips {
        if !sans.iter().any(|s| match s {
            rcgen::SanType::IpAddress(existing) => existing == &ip,
            _ => false,
        }) {
            sans.push(rcgen::SanType::IpAddress(ip));
        }
    }

    params.subject_alt_names = sans;

    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let cert = params.self_signed(&key_pair)?;
    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();
    Ok((cert_pem, key_pem))
}

/// Generates a self-signed X.509 certificate and private key in PEM format using rcgen.
pub fn generate_self_signed_cert_pem() -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error + Send + Sync>> {
    generate_self_signed_cert_pem_with_ips(&[])
}

/// Creates RustlsConfig from generated self-signed certificate PEM bytes with optional extra IPs.
pub async fn create_rustls_config_with_ips(extra_ips: &[std::net::IpAddr]) -> Result<RustlsConfig, Box<dyn Error + Send + Sync>> {
    let (cert_pem, key_pem) = generate_self_signed_cert_pem_with_ips(extra_ips)?;
    let config = RustlsConfig::from_pem(cert_pem, key_pem).await?;
    Ok(config)
}

/// Creates RustlsConfig from generated self-signed certificate PEM bytes.
pub async fn create_rustls_config() -> Result<RustlsConfig, Box<dyn Error + Send + Sync>> {
    create_rustls_config_with_ips(&[]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_local_ips() {
        let ips = discover_local_ips();
        println!("Discovered local IPs: {:?}", ips);
        for ip in &ips {
            assert!(!ip.is_loopback(), "Discovered IP should not be loopback: {:?}", ip);
            assert!(!ip.is_unspecified(), "Discovered IP should not be unspecified: {:?}", ip);
        }
    }

    #[test]
    fn test_generate_self_signed_cert_pem_with_extra_ips() {
        let extra_ip: std::net::IpAddr = "192.168.1.100".parse().unwrap();
        let res = generate_self_signed_cert_pem_with_ips(&[extra_ip]);
        assert!(res.is_ok(), "Failed to generate cert with extra IPs: {:?}", res.err());
        let (cert, key) = res.unwrap();
        assert!(!cert.is_empty());
        assert!(!key.is_empty());
    }

    #[test]
    fn test_generate_self_signed_cert_pem() {
        let res = generate_self_signed_cert_pem();
        assert!(res.is_ok(), "Failed to generate self-signed cert: {:?}", res.err());
        let (cert, key) = res.unwrap();
        assert!(!cert.is_empty());
        assert!(!key.is_empty());
        let cert_str = String::from_utf8(cert).unwrap();
        let key_str = String::from_utf8(key).unwrap();
        assert!(cert_str.contains("BEGIN CERTIFICATE"));
        assert!(key_str.contains("PRIVATE KEY"));
    }

    #[tokio::test]
    async fn test_create_rustls_config() {
        let res = create_rustls_config().await;
        assert!(res.is_ok(), "Failed to create rustls config: {:?}", res.err());
    }

    #[tokio::test]
    async fn test_https_server_bind_and_serve() {
        let tls_config = create_rustls_config().await.unwrap();
        let app = axum::Router::new().route("/ping", axum::routing::get(|| async { "pong" }));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();

        let server_task = tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls_config)
                .serve(app.into_make_service())
                .await
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        server_task.abort();
    }
}
