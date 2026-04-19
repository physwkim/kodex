use std::net::{IpAddr, ToSocketAddrs};

const ALLOWED_SCHEMES: &[&str] = &["http", "https"];
const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.google.com"];

/// Validate a URL: must be http/https, must not target private/internal IPs.
///
/// Blocks file://, ftp://, data:, and any scheme that could be used for SSRF
/// or local file access. Also blocks private/reserved IP ranges and cloud
/// metadata endpoints.
pub fn validate_url(url: &str) -> crate::error::Result<String> {
    // Parse scheme
    let scheme_end = url.find("://").ok_or_else(|| {
        crate::error::KodexError::UrlValidation(format!("Invalid URL (no scheme): {url:?}"))
    })?;
    let scheme = &url[..scheme_end].to_lowercase();

    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(crate::error::KodexError::UrlValidation(format!(
            "Blocked URL scheme '{scheme}' - only http and https are allowed. Got: {url:?}"
        )));
    }

    // Extract hostname
    let after_scheme = &url[scheme_end + 3..];
    let host_part = after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .split('@')
        .next_back()
        .unwrap_or(after_scheme);

    // Strip port if present
    let hostname = if host_part.starts_with('[') {
        // IPv6
        host_part
            .split(']')
            .next()
            .unwrap_or(host_part)
            .trim_start_matches('[')
    } else {
        host_part.split(':').next().unwrap_or(host_part)
    };

    if hostname.is_empty() {
        return Err(crate::error::KodexError::UrlValidation(format!(
            "Empty hostname in URL: {url:?}"
        )));
    }

    // Block known cloud metadata hostnames
    if BLOCKED_HOSTS.contains(&hostname.to_lowercase().as_str()) {
        return Err(crate::error::KodexError::UrlValidation(format!(
            "Blocked cloud metadata endpoint '{hostname}'. Got: {url:?}"
        )));
    }

    // Resolve hostname and block private/reserved IP ranges
    let addr_str = format!("{hostname}:0");
    if let Ok(addrs) = addr_str.to_socket_addrs() {
        for addr in addrs {
            let ip = addr.ip();
            if is_blocked_ip(&ip) {
                return Err(crate::error::KodexError::UrlValidation(format!(
                    "Blocked private/internal IP {ip} (resolved from '{hostname}'). Got: {url:?}"
                )));
            }
        }
    }
    // DNS failure will surface later during fetch

    Ok(url.to_string())
}

fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80   // fe80::/10 link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00   // fc00::/7 unique local
                || (v6.segments()[0] & 0xff00) == 0xff00 // ff00::/8 multicast
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_url() {
        assert!(validate_url("https://example.com/page").is_ok());
        assert!(validate_url("http://example.com").is_ok());
    }

    #[test]
    fn test_blocked_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("data:text/plain,hello").is_err());
    }

    #[test]
    fn test_blocked_metadata() {
        assert!(validate_url("http://metadata.google.internal/").is_err());
    }

    #[test]
    fn test_blocked_private_ip() {
        assert!(validate_url("http://127.0.0.1/").is_err());
        assert!(validate_url("http://10.0.0.1/").is_err());
    }
}
