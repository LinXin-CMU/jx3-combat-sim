//! Outbound policy for user-owned endpoints (configured server profiles are unchanged).
use super::{config::is_loopback_host, ProviderError};
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    Client, Url,
};
use std::{net::IpAddr, sync::Arc, time::Duration};

pub(super) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2))))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Globally routed unicast only; reject mapped IPv4, transition,
            // documentation, link-local, private and special-purpose ranges.
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

pub(super) fn normalize_url(input: &str, allow_loopback: bool) -> Result<String, ProviderError> {
    let invalid = || {
        ProviderError::configuration(
            "custom_url_invalid",
            "请填写完整的 API 地址，不含账号、查询参数或片段",
        )
    };
    if input.len() > 2048 {
        return Err(invalid());
    }
    let mut url = Url::parse(input.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(invalid());
    }
    let host = url.host_str().unwrap();
    let local = allow_loopback && is_loopback_host(host);
    if url.scheme() != "https" && !local {
        return Err(ProviderError::configuration(
            "custom_https_required",
            "远程 API 地址请使用 HTTPS",
        ));
    }
    if !local
        && (is_loopback_host(host)
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| !public_ip(ip)))
    {
        return Err(ProviderError::configuration(
            "custom_address_blocked",
            "此地址不能用于自定义接口",
        ));
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/responses"] {
        if let Some(base) = path.strip_suffix(suffix) {
            path = base.to_string();
            break;
        }
    }
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

struct PublicResolver {
    allow_loopback: bool,
}
impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let local = self.allow_loopback && is_loopback_host(name.as_str());
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            // Validate the exact addresses handed to the connector on every DNS
            // resolution, rather than preflighting DNS and resolving again later.
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|a| !(public_ip(a.ip()) || local && a.ip().is_loopback()))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "provider address blocked",
                )
                .into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

pub(super) fn client(allow_loopback: bool) -> Result<Client, ProviderError> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(240))
        .dns_resolver(Arc::new(PublicResolver { allow_loopback }))
        .build()
        .map_err(|_| {
            ProviderError::configuration("custom_client_unavailable", "接口连接器初始化失败")
        })
}
