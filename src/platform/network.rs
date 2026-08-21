use std::{
    env, fmt, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    time::Duration,
};

use reqwest::{
    Url,
    blocking::{Client, ClientBuilder},
    header::{CONTENT_LENGTH, LOCATION},
    redirect::Policy,
};

pub const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_REDIRECT_LIMIT: usize = 5;
const CLASH_USER_AGENT: &str = "mihomo/1.19 mh-provider";
const SING_BOX_USER_AGENT: &str = "sb/1.0 sing-box-provider";

pub trait ProfileSource {
    fn download(&self, url: &str) -> Result<Vec<u8>, DownloadError>;

    fn download_with_route(&self, url: &str) -> Result<(Vec<u8>, DownloadVia), DownloadError> {
        let content = self.download(url)?;
        Ok((content, self.download_via(url)))
    }

    fn download_via(&self, _url: &str) -> DownloadVia {
        DownloadVia::Direct
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadVia {
    Direct,
    Proxy,
    Unknown,
}

impl DownloadVia {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Proxy => "proxy",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadError(String);

impl DownloadError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DownloadError {}

#[derive(Debug, Clone)]
pub struct SecureDownloader {
    connect_timeout: Duration,
    request_timeout: Duration,
    max_bytes: usize,
    redirect_limit: usize,
    user_agent: String,
}

impl Default for SecureDownloader {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            redirect_limit: DEFAULT_REDIRECT_LIMIT,
            user_agent: CLASH_USER_AGENT.into(),
        }
    }
}

impl SecureDownloader {
    pub fn with_limits(
        connect_timeout: Duration,
        request_timeout: Duration,
        max_bytes: usize,
        redirect_limit: usize,
    ) -> Self {
        Self {
            connect_timeout,
            request_timeout,
            max_bytes,
            redirect_limit,
            user_agent: CLASH_USER_AGENT.into(),
        }
    }

    pub fn for_family(family: &str) -> Self {
        Self {
            user_agent: match family {
                "sing-box" => SING_BOX_USER_AGENT,
                _ => CLASH_USER_AGENT,
            }
            .into(),
            ..Self::default()
        }
    }

    fn client_for(&self, url: &Url, proxy: Option<&str>) -> Result<Client, DownloadError> {
        let host = validated_host(url)?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| DownloadError::new("download URL has no usable port"))?;
        let addresses = resolve_public(&host, port)?;
        let socket_addresses: Vec<_> = addresses
            .into_iter()
            .map(|address| SocketAddr::new(address, port))
            .collect();

        let mut builder = ClientBuilder::new()
            // Match mh's curl behavior while keeping proxy selection explicit.
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .resolve_to_addrs(&host, &socket_addresses)
            .user_agent(&self.user_agent);
        if let Some(proxy) = proxy {
            let proxy = reqwest::Proxy::all(proxy)
                .map_err(|_| DownloadError::new("invalid download proxy"))?;
            builder = builder.proxy(proxy);
        }
        builder
            .build()
            .map_err(|_| DownloadError::new("failed to initialize secure downloader"))
    }

    fn parse_url(value: &str) -> Result<Url, DownloadError> {
        let url = Url::parse(value).map_err(|_| DownloadError::new("invalid download URL"))?;
        validate_url(&url)?;
        Ok(url)
    }
}

impl ProfileSource for SecureDownloader {
    fn download(&self, value: &str) -> Result<Vec<u8>, DownloadError> {
        self.download_with_route(value).map(|(content, _)| content)
    }

    fn download_with_route(&self, value: &str) -> Result<(Vec<u8>, DownloadVia), DownloadError> {
        let parsed = Self::parse_url(value)?;
        let configured_proxy = proxy_for(&parsed);
        let candidates = configured_proxy.as_deref().map_or_else(
            || vec![(DownloadVia::Direct, None)],
            |proxy| {
                vec![
                    (DownloadVia::Proxy, Some(proxy)),
                    (DownloadVia::Direct, None),
                ]
            },
        );
        let mut last_error = None;
        for (route, proxy) in candidates {
            match self.download_once(value, proxy) {
                Ok(content) => return Ok((content, route)),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| DownloadError::new("download failed")))
    }

    fn download_via(&self, value: &str) -> DownloadVia {
        Self::parse_url(value)
            .ok()
            .and_then(|url| proxy_for(&url))
            .map(|_| DownloadVia::Proxy)
            .unwrap_or(DownloadVia::Direct)
    }
}

impl SecureDownloader {
    fn download_once(&self, value: &str, proxy: Option<&str>) -> Result<Vec<u8>, DownloadError> {
        let mut url = Self::parse_url(value)?;
        for redirects in 0..=self.redirect_limit {
            let redirect_proxy = proxy.and_then(|_| proxy_for(&url));
            let client = self.client_for(&url, redirect_proxy.as_deref())?;
            let mut response = client.get(url.clone()).send().map_err(|_| {
                DownloadError::new(format!("request to {} failed", redact_url(&url)))
            })?;

            if response.status().is_redirection() {
                if redirects == self.redirect_limit {
                    return Err(DownloadError::new("download redirect limit exceeded"));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| DownloadError::new("redirect response has no valid Location"))?;
                url = url
                    .join(location)
                    .map_err(|_| DownloadError::new("redirect has an invalid Location"))?;
                validate_url(&url)?;
                continue;
            }

            if !response.status().is_success() {
                return Err(DownloadError::new(format!(
                    "request to {} returned HTTP {}",
                    redact_url(&url),
                    response.status().as_u16()
                )));
            }
            if response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|length| length > self.max_bytes as u64)
            {
                return Err(DownloadError::new("download exceeds maximum allowed size"));
            }

            let mut content = Vec::new();
            io::Read::read_to_end(
                &mut io::Read::take(&mut response, self.max_bytes as u64 + 1),
                &mut content,
            )
            .map_err(|_| DownloadError::new("failed while reading download response"))?;
            if content.len() > self.max_bytes {
                return Err(DownloadError::new("download exceeds maximum allowed size"));
            }
            return Ok(content);
        }
        unreachable!("redirect loop always returns")
    }
}

fn proxy_for(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let no_proxy = env::var("NO_PROXY").or_else(|_| env::var("no_proxy")).ok();
    if no_proxy
        .as_deref()
        .is_some_and(|items| no_proxy_matches(items, host))
    {
        return None;
    }

    let keys = if url.scheme() == "https" {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    keys.into_iter()
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
}

fn no_proxy_matches(value: &str, host: &str) -> bool {
    value.split(',').map(str::trim).any(|item| {
        if item.is_empty() {
            return false;
        }
        if item == "*" {
            return true;
        }
        let item = item
            .rsplit_once(':')
            .filter(|(_, suffix)| suffix.parse::<u16>().is_ok())
            .map_or(item, |(host, _)| host);
        let item = item.trim_start_matches('.').trim_matches(['[', ']']);
        let host = host.trim_matches(['[', ']']);
        host.eq_ignore_ascii_case(item) || host.to_ascii_lowercase().ends_with(&format!(".{item}"))
    })
}

pub fn validate_url(url: &Url) -> Result<(), DownloadError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DownloadError::new("only HTTP(S) download URLs are allowed"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DownloadError::new(
            "download URL must not include user credentials",
        ));
    }
    if url.host_str().is_none() {
        return Err(DownloadError::new("download URL must include a host"));
    }
    validated_host(url).map(|_| ())
}

fn validated_host(url: &Url) -> Result<String, DownloadError> {
    let host = url
        .host_str()
        .ok_or_else(|| DownloadError::new("download URL must include a host"))?;
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return Err(DownloadError::new("download host is not publicly routable"));
    }
    if let Ok(address) = normalized.parse::<IpAddr>()
        && !is_public_ip(address)
    {
        return Err(DownloadError::new("download host is not publicly routable"));
    }
    Ok(normalized)
}

fn resolve_public(host: &str, port: u16) -> Result<Vec<IpAddr>, DownloadError> {
    let addresses: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|_| DownloadError::new("download host DNS resolution failed"))?
        .map(|address| address.ip())
        .collect();
    if addresses.is_empty() {
        return Err(DownloadError::new(
            "download host DNS returned no addresses",
        ));
    }
    if addresses.iter().any(|address| !is_public_ip(*address)) {
        return Err(DownloadError::new(
            "download host DNS returned a non-public address",
        ));
    }
    Ok(addresses)
}

pub fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_v4(address),
        IpAddr::V6(address) => is_public_v6(address),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_private()
        && !address.is_link_local()
        && !address.is_multicast()
        && !address.is_broadcast()
        && !in_v4(value, [0, 0, 0, 0], 8)
        && !in_v4(value, [100, 64, 0, 0], 10)
        && !in_v4(value, [192, 0, 0, 0], 24)
        && !in_v4(value, [192, 0, 2, 0], 24)
        && !in_v4(value, [198, 18, 0, 0], 15)
        && !in_v4(value, [198, 51, 100, 0], 24)
        && !in_v4(value, [203, 0, 113, 0], 24)
        && !in_v4(value, [240, 0, 0, 0], 4)
}

fn in_v4(value: u32, network: [u8; 4], prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    value & mask == u32::from(Ipv4Addr::from(network)) & mask
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let segments = address.segments();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || segments[0] & 0xfe00 == 0xfc00
        || segments[0] & 0xffc0 == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0))
}

pub fn redact_url(url: &Url) -> String {
    let host = url.host_str().unwrap_or("<invalid-host>");
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{host}{port}/<redacted>", url.scheme())
}

#[cfg(test)]
mod tests {
    use super::{
        CLASH_USER_AGENT, SING_BOX_USER_AGENT, SecureDownloader, is_public_ip, redact_url,
        validate_url,
    };
    use reqwest::Url;
    use std::net::IpAddr;

    #[test]
    fn rejects_local_private_reserved_and_documentation_addresses() {
        for address in [
            "0.1.2.3",
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                !is_public_ip(address.parse::<IpAddr>().unwrap()),
                "{address}"
            );
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn rejects_non_http_and_localhost_urls() {
        assert!(validate_url(&Url::parse("file:///etc/passwd").unwrap()).is_err());
        assert!(validate_url(&Url::parse("http://localhost/sub").unwrap()).is_err());
        assert!(validate_url(&Url::parse("https://api.localhost./sub").unwrap()).is_err());
        assert!(validate_url(&Url::parse("https://127.0.0.1/sub").unwrap()).is_err());
        assert!(validate_url(&Url::parse("https://user:pass@example.com/sub").unwrap()).is_err());
    }

    #[test]
    fn redaction_removes_credentials_path_query_and_fragment() {
        let url = Url::parse("https://user:pass@example.com:8443/private?token=secret#x").unwrap();
        let redacted = redact_url(&url);
        assert_eq!(redacted, "https://example.com:8443/<redacted>");
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("user"));
    }

    #[test]
    fn selects_provider_user_agent_by_family() {
        assert_eq!(
            SecureDownloader::for_family("clash").user_agent,
            CLASH_USER_AGENT
        );
        assert_eq!(
            SecureDownloader::for_family("sing-box").user_agent,
            SING_BOX_USER_AGENT
        );
    }
}
