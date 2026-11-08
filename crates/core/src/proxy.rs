//! Proxy selection for outgoing HTTP clients.
//!
//! reqwest's `system-proxy` feature honours the proxy environment variables
//! and, on Windows, the manual proxy from Internet Settings. It ignores the
//! two automatic modes Windows offers: "Use setup script" (a PAC file) and
//! "Automatically detect settings" (WPAD), which is how most corporate
//! networks hand out their proxy. A browser on the same machine reaches the
//! SAP system; the app gets "host unknown".
//!
//! On Windows, [`apply_system_proxy`] asks WinHTTP which proxy the system
//! would use for the target URL. WinHTTP downloads and runs the PAC script
//! itself, as browsers do, so no JavaScript engine is bundled here. The
//! answer is pinned on the client for that URL's host.
//!
//! Precedence, highest first:
//! 1. `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`: left to reqwest, unchanged.
//! 2. PAC script / WPAD, resolved through WinHTTP (Windows only). `NO_PROXY`
//!    still exempts hosts from the proxy it returns.
//! 3. The manual Windows proxy, or no proxy: left to reqwest, unchanged.
//!    Also the fallback when the PAC script can't be fetched or run.

use reqwest::ClientBuilder;

/// The route the system picked for one URL.
#[derive(Debug, Clone, PartialEq, Eq)]
// Only the Windows lookup produces routes.
#[cfg_attr(not(windows), allow(dead_code))]
enum ProxyRoute {
    /// Connect to the server directly (PAC `DIRECT`).
    Direct,
    /// Go through this proxy, as a URL (`http://host:port`).
    Proxy(String),
}

/// Proxy environment variables that reqwest reads on its own. When any is
/// set, the user chose the proxy explicitly and the system lookup is skipped.
const PROXY_ENV_VARS: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

fn proxy_env_is_set() -> bool {
    PROXY_ENV_VARS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// Route `builder`'s requests the way the operating system would route a
/// request to `target_url`. A no-op unless the system uses a PAC script or
/// WPAD (Windows only) and no proxy environment variable is set; see the
/// module docs for the precedence.
///
/// The route is looked up once, for `target_url`'s host, and applies to
/// every request the client sends. That fits a client that talks to one SAP
/// system; a redirect to another host takes the same route.
pub fn apply_system_proxy(builder: ClientBuilder, target_url: &str) -> ClientBuilder {
    if proxy_env_is_set() {
        return builder;
    }
    apply_route(builder, imp::resolve(target_url))
}

/// Pin `route` on `builder`; `None` leaves reqwest's own proxy lookup in
/// charge.
fn apply_route(builder: ClientBuilder, route: Option<ProxyRoute>) -> ClientBuilder {
    match route {
        Some(ProxyRoute::Direct) => builder.no_proxy(),
        Some(ProxyRoute::Proxy(proxy_url)) => match reqwest::Proxy::all(&proxy_url) {
            Ok(proxy) => builder.proxy(proxy.no_proxy(reqwest::NoProxy::from_env())),
            Err(e) => {
                tracing::warn!(
                    "ignoring the proxy '{proxy_url}' from the system proxy script: {e}"
                );
                builder
            }
        },
        None => builder,
    }
}

/// Pick the proxy for a request with scheme `target_scheme` (`http` /
/// `https`) from a WinHTTP proxy list and return it as a URL.
///
/// WinHTTP lists proxies separated by `;` or whitespace, each
/// `[<scheme>=][<scheme>://]<server>[:<port>]`. A `<scheme>=` entry applies
/// only to requests of that scheme and wins over a plain entry. SOCKS
/// proxies are skipped: reqwest is built without SOCKS support.
#[cfg(any(windows, test))]
fn pick_proxy(list: &str, target_scheme: &str) -> Option<String> {
    let mut generic = None;
    for entry in list.split(|c: char| c == ';' || c.is_whitespace()) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (for_scheme, server) = match entry.split_once('=') {
            Some((scheme, server)) => (Some(scheme.trim()), server.trim()),
            None => (None, entry),
        };
        let Some(url) = proxy_url(server) else {
            continue;
        };
        match for_scheme {
            Some(scheme) if scheme.eq_ignore_ascii_case(target_scheme) => return Some(url),
            Some(_) => {}
            None => {
                generic.get_or_insert(url);
            }
        }
    }
    generic
}

/// `server` as a proxy URL, or `None` when it names a proxy reqwest can't
/// use (SOCKS) or is empty.
#[cfg(any(windows, test))]
fn proxy_url(server: &str) -> Option<String> {
    if server.is_empty() {
        return None;
    }
    match server.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            (matches!(scheme.as_str(), "http" | "https") && !rest.is_empty())
                .then(|| format!("{scheme}://{rest}"))
        }
        None => Some(format!("http://{server}")),
    }
}

#[cfg(windows)]
mod imp {
    use super::{ProxyRoute, pick_proxy};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
    use windows::Win32::Networking::WinHttp::{
        ERROR_WINHTTP_AUTO_PROXY_SERVICE_ERROR, ERROR_WINHTTP_AUTODETECTION_FAILED,
        ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT, ERROR_WINHTTP_LOGIN_FAILURE, ERROR_WINHTTP_TIMEOUT,
        ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT, WINHTTP_ACCESS_TYPE_NAMED_PROXY,
        WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTO_DETECT_TYPE_DHCP,
        WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_AUTOPROXY_AUTO_DETECT,
        WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS,
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO, WinHttpCloseHandle,
        WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
        WinHttpSetTimeouts,
    };
    use windows::core::{BOOL, HRESULT, PCWSTR, PWSTR, w};

    /// How long a lookup is reused. The desktop app builds a client per
    /// command; without the cache a network with "Automatically detect
    /// settings" on and no WPAD server would pay the failed discovery on
    /// every click. Short enough that connecting or dropping a VPN, which
    /// can change the PAC answer, takes effect within a minute.
    const CACHE_TTL: Duration = Duration::from_secs(60);

    /// Timeout for each step (resolve, connect, send, receive) of fetching
    /// the PAC script, so an unreachable script server can't stall a request
    /// by WinHTTP's default of a minute or more.
    const PAC_FETCH_TIMEOUT_MS: i32 = 5_000;

    type Cache = Mutex<HashMap<String, (Instant, Option<ProxyRoute>)>>;

    fn cache() -> &'static Cache {
        static CACHE: OnceLock<Cache> = OnceLock::new();
        CACHE.get_or_init(Default::default)
    }

    pub(super) fn resolve(target_url: &str) -> Option<ProxyRoute> {
        let url = url::Url::parse(target_url).ok()?;
        let origin = url.origin().ascii_serialization();

        // Read the settings on every call (a registry read): it is how a
        // switch from "setup script" to "no proxy" takes effect without the
        // cache hiding it.
        let settings = auto_proxy_settings()?;

        let key = format!(
            "{origin}|{}|{}",
            settings.auto_detect,
            settings.config_url.as_deref().unwrap_or("")
        );
        if let Ok(cache) = cache().lock()
            && let Some((at, route)) = cache.get(&key)
            && at.elapsed() < CACHE_TTL
        {
            return route.clone();
        }

        let started = Instant::now();
        let (route, outcome) = match unsafe { proxy_for_url(url.as_str(), &settings) } {
            Ok(Some(list)) => match pick_proxy(&list, url.scheme()) {
                Some(proxy) => (Some(ProxyRoute::Proxy(proxy.clone())), proxy),
                None => {
                    tracing::warn!(
                        "the system proxy script returned no usable proxy for {origin} \
                         ('{list}'); using the default proxy settings"
                    );
                    (None, format!("no usable proxy in '{list}'"))
                }
            },
            Ok(None) => (Some(ProxyRoute::Direct), "direct".to_string()),
            Err(e) => (None, format!("lookup failed: {}", describe_error(&e))),
        };
        tracing::debug!(
            "system proxy for {origin}: {outcome}{} (auto-detect: {}, setup script: {}, {} ms)",
            if route.is_none() {
                "; using the default proxy settings"
            } else {
                ""
            },
            settings.auto_detect,
            settings.config_url.as_deref().unwrap_or("none"),
            started.elapsed().as_millis(),
        );

        if let Ok(mut cache) = cache().lock() {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            cache.insert(key, (Instant::now(), route.clone()));
        }
        route
    }

    #[derive(Clone)]
    struct AutoProxySettings {
        auto_detect: bool,
        config_url: Option<String>,
    }

    /// The current user's automatic proxy settings, or `None` when neither
    /// a setup script nor auto-detection is on (or they can't be read).
    fn auto_proxy_settings() -> Option<AutoProxySettings> {
        let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
        unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) }.ok()?;
        // The manual proxy (`lpszProxy`, `lpszProxyBypass`) is reqwest's
        // job; only the strings need freeing.
        let config_url = unsafe { take_wide(config.lpszAutoConfigUrl) };
        unsafe {
            take_wide(config.lpszProxy);
            take_wide(config.lpszProxyBypass);
        }
        let config_url = config_url.filter(|u| !u.trim().is_empty());
        let auto_detect = config.fAutoDetect.as_bool();
        (auto_detect || config_url.is_some()).then_some(AutoProxySettings {
            auto_detect,
            config_url,
        })
    }

    /// Ask WinHTTP for the proxy list for `url`. `Ok(None)` means direct.
    unsafe fn proxy_for_url(
        url: &str,
        settings: &AutoProxySettings,
    ) -> windows::core::Result<Option<String>> {
        let session = Session::open()?;

        let config_url_wide: Option<Vec<u16>> = settings
            .config_url
            .as_deref()
            .map(|u| u.encode_utf16().chain(std::iter::once(0)).collect());
        let url_wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();

        let mut options = WINHTTP_AUTOPROXY_OPTIONS::default();
        if settings.auto_detect {
            options.dwFlags |= WINHTTP_AUTOPROXY_AUTO_DETECT;
            options.dwAutoDetectFlags =
                WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
        }
        if let Some(wide) = &config_url_wide {
            options.dwFlags |= WINHTTP_AUTOPROXY_CONFIG_URL;
            options.lpszAutoConfigUrl = PCWSTR(wide.as_ptr());
        }

        let mut info = WINHTTP_PROXY_INFO::default();
        let mut result = unsafe {
            WinHttpGetProxyForUrl(
                session.0,
                PCWSTR(url_wide.as_ptr()),
                &mut options,
                &mut info,
            )
        };
        // Microsoft's documented pattern: try without credentials first,
        // and only send the Windows logon when the script server asks.
        if let Err(e) = &result
            && e.code() == HRESULT::from_win32(ERROR_WINHTTP_LOGIN_FAILURE)
        {
            options.fAutoLogonIfChallenged = BOOL::from(true);
            result = unsafe {
                WinHttpGetProxyForUrl(
                    session.0,
                    PCWSTR(url_wide.as_ptr()),
                    &mut options,
                    &mut info,
                )
            };
        }
        result?;

        let list = unsafe { take_wide(info.lpszProxy) };
        unsafe { take_wide(info.lpszProxyBypass) };
        if info.dwAccessType == WINHTTP_ACCESS_TYPE_NAMED_PROXY {
            Ok(list.filter(|l| !l.trim().is_empty()))
        } else if info.dwAccessType == WINHTTP_ACCESS_TYPE_NO_PROXY {
            Ok(None)
        } else {
            Err(windows::core::Error::new(
                HRESULT(0x8000_4005_u32 as i32), // E_FAIL
                format!("unexpected access type {}", info.dwAccessType.0),
            ))
        }
    }

    /// A readable reason for the failures a lookup commonly hits. The
    /// system message table has no text for WinHTTP codes, so `Display`
    /// would print only the bare HRESULT.
    fn describe_error(e: &windows::core::Error) -> String {
        let reason = [
            (
                ERROR_WINHTTP_AUTODETECTION_FAILED,
                "no proxy auto-detection (WPAD) server found",
            ),
            (
                ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT,
                "could not download the proxy setup script",
            ),
            (
                ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT,
                "the proxy setup script failed to run",
            ),
            (
                ERROR_WINHTTP_AUTO_PROXY_SERVICE_ERROR,
                "the WinHTTP auto-proxy service failed",
            ),
            (
                ERROR_WINHTTP_LOGIN_FAILURE,
                "the setup script server refused the Windows logon",
            ),
            (ERROR_WINHTTP_TIMEOUT, "timed out"),
        ]
        .into_iter()
        .find(|(code, _)| e.code() == HRESULT::from_win32(*code))
        .map(|(_, reason)| reason);
        match reason {
            Some(reason) => format!("{reason} ({})", e.code()),
            None => e.to_string(),
        }
    }

    /// A WinHTTP session handle, closed on drop.
    struct Session(*mut core::ffi::c_void);

    impl Session {
        fn open() -> windows::core::Result<Self> {
            // `NO_PROXY` access: the PAC script itself is fetched directly.
            let handle = unsafe {
                WinHttpOpen(
                    w!("sap-odata-explorer"),
                    WINHTTP_ACCESS_TYPE_NO_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                )
            };
            if handle.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            let session = Self(handle);
            unsafe {
                WinHttpSetTimeouts(
                    session.0,
                    PAC_FETCH_TIMEOUT_MS,
                    PAC_FETCH_TIMEOUT_MS,
                    PAC_FETCH_TIMEOUT_MS,
                    PAC_FETCH_TIMEOUT_MS,
                )
            }?;
            Ok(session)
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }

    /// Copy a WinHTTP-allocated wide string and free it (`GlobalFree`, as
    /// the WinHTTP docs require). Null gives `None`.
    unsafe fn take_wide(p: PWSTR) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let s = unsafe { p.to_string() }.ok();
        let _ = unsafe { GlobalFree(Some(HGLOBAL(p.0.cast()))) };
        s
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const PAC: &str = r#"function FindProxyForURL(url, host) {
            if (host == "sap.example.test") return "PROXY 127.0.0.1:3128; DIRECT";
            if (host == "split.example.test") return "PROXY p1.example.test:8080; PROXY p2.example.test:8080";
            return "DIRECT";
        }"#;

        /// Real WinHTTP evaluation of a PAC script, served locally and
        /// passed in directly: the user's proxy settings are not read or
        /// changed.
        #[tokio::test(flavor = "multi_thread")]
        async fn winhttp_evaluates_a_pac_script() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/proxy.pac"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("Content-Type", "application/x-ns-proxy-autoconfig")
                        .set_body_string(PAC),
                )
                .mount(&server)
                .await;
            let settings = AutoProxySettings {
                auto_detect: false,
                config_url: Some(format!("{}/proxy.pac", server.uri())),
            };

            let lookup = move |url: &'static str| {
                let settings = settings.clone();
                tokio::task::spawn_blocking(move || unsafe { proxy_for_url(url, &settings) })
            };

            let proxied = lookup("https://sap.example.test:44300/sap/opu/odata/")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                proxied
                    .as_deref()
                    .and_then(|l| pick_proxy(l, "https"))
                    .as_deref(),
                Some("http://127.0.0.1:3128")
            );

            let split = lookup("http://split.example.test/").await.unwrap().unwrap();
            assert_eq!(
                split
                    .as_deref()
                    .and_then(|l| pick_proxy(l, "http"))
                    .as_deref(),
                Some("http://p1.example.test:8080")
            );

            let direct = lookup("https://other.example.test/")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(direct, None);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ProxyRoute;

    /// macOS and Linux: reqwest's own lookup (environment variables, and the
    /// macOS system proxy) stays in charge.
    pub(super) fn resolve(_target_url: &str) -> Option<ProxyRoute> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_entry_gets_http_scheme() {
        assert_eq!(
            pick_proxy("proxy.corp:8080", "https").as_deref(),
            Some("http://proxy.corp:8080")
        );
    }

    #[test]
    fn first_of_several_plain_entries_wins() {
        assert_eq!(
            pick_proxy("p1.corp:8080; p2.corp:3128", "https").as_deref(),
            Some("http://p1.corp:8080")
        );
        assert_eq!(
            pick_proxy("p1.corp:8080 p2.corp:3128", "http").as_deref(),
            Some("http://p1.corp:8080")
        );
    }

    #[test]
    fn scheme_specific_entry_matches_target_scheme() {
        let list = "http=web.corp:80;https=secure.corp:443";
        assert_eq!(
            pick_proxy(list, "https").as_deref(),
            Some("http://secure.corp:443")
        );
        assert_eq!(
            pick_proxy(list, "http").as_deref(),
            Some("http://web.corp:80")
        );
    }

    #[test]
    fn scheme_specific_entry_beats_earlier_plain_entry() {
        assert_eq!(
            pick_proxy("any.corp:8080;https=secure.corp:443", "HTTPS").as_deref(),
            Some("http://secure.corp:443")
        );
    }

    #[test]
    fn entry_for_another_scheme_only_gives_nothing() {
        assert_eq!(pick_proxy("ftp=ftp.corp:21", "https"), None);
    }

    #[test]
    fn explicit_scheme_is_kept() {
        assert_eq!(
            pick_proxy("https://tls-proxy.corp:8443", "https").as_deref(),
            Some("https://tls-proxy.corp:8443")
        );
        assert_eq!(
            pick_proxy("HTTP://proxy.corp:8080", "https").as_deref(),
            Some("http://proxy.corp:8080")
        );
    }

    #[test]
    fn socks_entries_are_skipped() {
        assert_eq!(
            pick_proxy("socks=sock.corp:1080;proxy.corp:8080", "https").as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(pick_proxy("socks5://sock.corp:1080", "https"), None);
    }

    #[test]
    fn empty_list_gives_nothing() {
        assert_eq!(pick_proxy("", "https"), None);
        assert_eq!(pick_proxy(" ; ", "https"), None);
    }

    /// The picked route really carries the traffic: a mock server stands in
    /// for the proxy and receives the request for the (unresolvable) host.
    #[tokio::test]
    async fn proxy_route_sends_requests_through_the_proxy() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let proxy = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sap/opu/odata/sap/ZTEST_SRV/$metadata"))
            .and(header("host", "sap.example.invalid"))
            .respond_with(ResponseTemplate::new(200).set_body_string("via proxy"))
            .expect(1)
            .mount(&proxy)
            .await;

        let client = apply_route(
            reqwest::Client::builder(),
            Some(ProxyRoute::Proxy(proxy.uri())),
        )
        .build()
        .unwrap();
        let body = client
            .get("http://sap.example.invalid/sap/opu/odata/sap/ZTEST_SRV/$metadata")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "via proxy");
    }

    #[test]
    fn picked_urls_are_valid_reqwest_proxies() {
        for list in [
            "proxy.corp:8080",
            "https=secure.corp:443",
            "https://tls.corp:8443",
        ] {
            let url = pick_proxy(list, "https").unwrap();
            assert!(reqwest::Proxy::all(&url).is_ok(), "{url}");
        }
    }
}
