//! One HTTP/2 connection per LINE gateway host.
//!
//! The inbox `sync` stream stays open, and a send is another stream on that
//! same TLS session. Callers block on their own thread; a small runtime drives
//! the socket so one side can write while the other is still waiting.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use h2::client::SendRequest;
use http::{HeaderName, HeaderValue, Request};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio::sync::Mutex as AsyncMutex;
use tokio_rustls::TlsConnector;

struct Gate {
    runtime: Runtime,
    slots: AsyncMutex<HashMap<String, Slot>>,
    dials: AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    generation: AtomicU64,
    dial_count: AtomicU64,
}

struct Slot {
    generation: u64,
    send: SendRequest<Bytes>,
}

struct Live {
    generation: u64,
    send: SendRequest<Bytes>,
}

struct Broken {
    message: String,
    reconnect: bool,
}

fn gate() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| {
        let runtime = Runtime::new().expect("http2 runtime");
        Gate {
            runtime,
            slots: AsyncMutex::new(HashMap::new()),
            dials: AsyncMutex::new(HashMap::new()),
            generation: AtomicU64::new(1),
            dial_count: AtomicU64::new(0),
        }
    })
}

pub fn post(
    host: &str,
    path: &str,
    body: &[u8],
    headers: &[(String, String)],
    timeout: Duration,
) -> Result<(u16, Vec<u8>, HashMap<String, String>), String> {
    if host.is_empty() || !path.starts_with('/') {
        return Err(format!("{host}{path}: missing gateway target"));
    }
    let host = host.to_string();
    let path = path.to_string();
    let body = body.to_vec();
    let headers = headers.to_vec();
    let gate = gate();
    gate.runtime.handle().block_on(async move {
        match tokio::time::timeout(timeout, exchange(&host, &path, body, &headers)).await {
            Ok(result) => result,
            Err(_) => Err(format!("timeout talking to {host}{path}")),
        }
    })
}

async fn exchange(host: &str, path: &str, body: Vec<u8>, headers: &[(String, String)]) -> Result<(u16, Vec<u8>, HashMap<String, String>), String> {
    let first = dispatch(host, path, &body, headers).await;
    let Err(broken) = first else {
        return first.map_err(|err| err.message);
    };
    if !broken.reconnect {
        return Err(broken.message);
    }
    forget(host, 0).await;
    dispatch(host, path, &body, headers).await.map_err(|err| err.message)
}

async fn dispatch(host: &str, path: &str, body: &[u8], headers: &[(String, String)]) -> Result<(u16, Vec<u8>, HashMap<String, String>), Broken> {
    let mut live = acquire(host).await.map_err(|message| Broken { message, reconnect: true })?;
    let result = send_stream(&mut live.send, host, path, body, headers).await;
    if let Err(broken) = &result {
        if broken.reconnect {
            forget(host, live.generation).await;
        }
    }
    result
}

async fn send_stream(
    send: &mut SendRequest<Bytes>,
    host: &str,
    path: &str,
    body: &[u8],
    headers: &[(String, String)],
) -> Result<(u16, Vec<u8>, HashMap<String, String>), Broken> {
    let request = build_request(host, path, headers).map_err(|message| Broken { message, reconnect: false })?;
    let (response, mut outbound) = send.send_request(request, body.is_empty()).map_err(|err| broken_io(host, path, &err))?;
    if !body.is_empty() {
        outbound
            .send_data(Bytes::copy_from_slice(body), true)
            .map_err(|err| broken_io(host, path, &err))?;
    }
    let response = response.await.map_err(|err| broken_io(host, path, &err))?;
    let status = response.status().as_u16();
    let mut header_map = HashMap::new();
    for (name, value) in response.headers().iter() {
        if let Ok(text) = value.to_str() {
            header_map.insert(name.as_str().to_ascii_lowercase(), text.to_string());
        }
    }
    let mut inbound = response.into_body();
    let mut payload = Vec::new();
    while let Some(chunk) = inbound.data().await {
        let chunk = chunk.map_err(|err| broken_io(host, path, &err))?;
        let len = chunk.len();
        payload.extend_from_slice(&chunk);
        let _ = inbound.flow_control().release_capacity(len);
    }
    Ok((status, payload, header_map))
}

fn build_request(host: &str, path: &str, headers: &[(String, String)]) -> Result<Request<()>, String> {
    let uri = format!("https://{host}{path}");
    let mut request = Request::builder()
        .method("POST")
        .uri(&uri)
        .body(())
        .map_err(|err| format!("{host}{path}: {err}"))?;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("keep-alive")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("host")
        {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else { continue };
        let Ok(value) = HeaderValue::from_str(value) else { continue };
        request.headers_mut().insert(name, value);
    }
    Ok(request)
}

fn broken_io(host: &str, path: &str, err: &h2::Error) -> Broken {
    Broken {
        message: format!("{host}{path}: {err}"),
        reconnect: err.is_io() || err.is_go_away(),
    }
}

async fn acquire(host: &str) -> Result<Live, String> {
    if let Some(live) = ready(host).await {
        return Ok(live);
    }
    let dial = dial_lock(host).await;
    let _guard = dial.lock().await;
    if let Some(live) = ready(host).await {
        return Ok(live);
    }
    establish(host).await
}

async fn ready(host: &str) -> Option<Live> {
    let (generation, send) = {
        let slots = gate().slots.lock().await;
        let slot = slots.get(host)?;
        (slot.generation, slot.send.clone())
    };
    match send.ready().await {
        Ok(send) => Some(Live { generation, send }),
        Err(_) => {
            forget(host, generation).await;
            None
        }
    }
}

async fn dial_lock(host: &str) -> Arc<AsyncMutex<()>> {
    let mut dials = gate().dials.lock().await;
    dials.entry(host.to_string()).or_insert_with(|| Arc::new(AsyncMutex::new(()))).clone()
}

async fn forget(host: &str, generation: u64) {
    let mut slots = gate().slots.lock().await;
    let stale = match slots.get(host) {
        Some(slot) => generation == 0 || slot.generation == generation,
        None => false,
    };
    if stale {
        slots.remove(host);
    }
}

async fn establish(host: &str) -> Result<Live, String> {
    let tls = dial_tls(host).await?;
    if tls.get_ref().1.alpn_protocol() != Some(b"h2") {
        return Err(format!("{host}: gateway did not negotiate http2"));
    }
    let (send, connection) = h2::client::Builder::new()
        .initial_window_size(1024 * 1024)
        .initial_connection_window_size(2 * 1024 * 1024)
        .handshake::<_, Bytes>(tls)
        .await
        .map_err(|err| format!("{host}: {err}"))?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let generation = gate().generation.fetch_add(1, Ordering::Relaxed);
    gate().dial_count.fetch_add(1, Ordering::Relaxed);
    gate().slots.lock().await.insert(host.to_string(), Slot { generation, send: send.clone() });
    Ok(Live { generation, send })
}

async fn dial_tls(host: &str) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let connector = TlsConnector::from(Arc::new(config));
    let server = ServerName::try_from(host.to_string()).map_err(|err| format!("{host}: {err}"))?;
    let stream = TcpStream::connect((host, 443)).await.map_err(|err| format!("{host}: {err}"))?;
    let _ = stream.set_nodelay(true);
    connector.connect(server, stream).await.map_err(|err| format!("{host}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_posts_share_one_http2_connection() {
        let host = "legy-jp.line-apps.com";
        let headers = vec![("content-type".into(), "application/x-thrift".into())];
        let gate = gate();
        let before = gate.dial_count.load(Ordering::Relaxed);
        let left_headers = headers.clone();
        let right_headers = headers;
        let left = std::thread::spawn(move || post(host, "/S4", b"\x80", &left_headers, Duration::from_secs(20)));
        let right = std::thread::spawn(move || post(host, "/S4", b"\x80", &right_headers, Duration::from_secs(20)));
        let left = left.join().expect("left post");
        let right = right.join().expect("right post");
        assert!(left.is_ok(), "{left:?}");
        assert!(right.is_ok(), "{right:?}");
        assert_eq!(gate.dial_count.load(Ordering::Relaxed) - before, 1);
    }
}
