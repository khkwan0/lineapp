use std::collections::BTreeMap;
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use url::{Host, Url};

const PAGE_LIMIT: usize = 256 * 1024;
const PREVIEW_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_CAP: usize = 64;

#[derive(Clone, serde::Serialize)]
pub struct Card {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image: String,
    pub site: String,
}

struct Cached {
    at: Instant,
    card: Card,
}

static CACHE: Mutex<Vec<(String, Cached)>> = Mutex::new(Vec::new());

pub fn open_in_browser(raw: &str) -> Result<(), String> {
    let url = parse_https(raw)?;
    let hint = browser_hint();
    focus_browser(&hint);
    std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("could not open that link: {err}"))?;
    if !hint.is_empty() {
        let later = hint;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            focus_browser(&later);
        });
    }
    Ok(())
}

fn browser_hint() -> String {
    let Ok(output) = std::process::Command::new("xdg-mime").args(["query", "default", "x-scheme-handler/https"]).output() else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    desktop_hint(&String::from_utf8_lossy(&output.stdout))
}

fn desktop_hint(desktop: &str) -> String {
    let name = desktop.trim().trim_end_matches(".desktop").to_ascii_lowercase();
    if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
        return String::new();
    }
    name
}

fn script_id(reply: &str) -> Option<u32> {
    let inner = reply.trim().strip_prefix('(')?.strip_suffix(",)")?;
    if inner.is_empty() || !inner.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    inner.parse().ok()
}

fn focus_browser(hint: &str) {
    if hint.is_empty() {
        return;
    }
    let script = format!(
        r#"var hint = "{hint}";
var stack = workspace.stackingOrder || workspace.windowList();
for (var i = stack.length - 1; i >= 0; i--) {{
  var win = stack[i];
  var desk = String(win.desktopFileName || "");
  var klass = String(win.resourceClass || "");
  if (desk === hint || klass === hint) {{
    win.minimized = false;
    workspace.activeWindow = win;
    break;
  }}
}}
"#
    );
    static FOCUS_SEQ: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "lineapp-focus-{}-{}",
        std::process::id(),
        FOCUS_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let path = std::env::temp_dir().join(format!("{name}.js"));
    if std::fs::write(&path, script).is_err() {
        return;
    }
    let path_text = path.to_string_lossy().to_string();
    let loaded = gdbus(&[
        "call",
        "--session",
        "--dest",
        "org.kde.KWin",
        "--object-path",
        "/Scripting",
        "--method",
        "org.kde.kwin.Scripting.loadScript",
        &path_text,
        &name,
    ]);
    if let Some(id) = loaded.as_deref().and_then(script_id) {
        let object = format!("/Scripting/Script{id}");
        let _ = gdbus(&[
            "call",
            "--session",
            "--dest",
            "org.kde.KWin",
            "--object-path",
            &object,
            "--method",
            "org.kde.kwin.Script.run",
        ]);
    }
    let _ = gdbus(&[
        "call",
        "--session",
        "--dest",
        "org.kde.KWin",
        "--object-path",
        "/Scripting",
        "--method",
        "org.kde.kwin.Scripting.unloadScript",
        &name,
    ]);
    let _ = std::fs::remove_file(path);
}

fn gdbus(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("gdbus").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn load(raw: &str) -> Result<Card, String> {
    let url = parse_https(raw)?;
    if !destination_public(&url) {
        return Err("that link cannot be previewed".into());
    }
    let key = url.to_string();
    if let Some(card) = fresh(&key) {
        return Ok(card);
    }
    let card = fetch(url);
    remember(key, card.clone());
    Ok(card)
}

fn fresh(key: &str) -> Option<Card> {
    let mut cache = CACHE.lock().ok()?;
    cache.retain(|(_, cached)| cached.at.elapsed() < PREVIEW_TTL);
    cache.iter().find(|(saved, _)| saved == key).map(|(_, cached)| cached.card.clone())
}

fn remember(key: String, card: Card) {
    let Ok(mut cache) = CACHE.lock() else { return };
    cache.retain(|(saved, _)| saved != &key);
    if cache.len() >= CACHE_CAP {
        cache.remove(0);
    }
    cache.push((key, Cached { at: Instant::now(), card }));
}

fn parse_https(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.len() > 2048 || raw.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err("that link cannot be opened".into());
    }
    let url = Url::parse(raw).map_err(|_| "that link cannot be opened".to_string())?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err("that link cannot be opened".into());
    }
    match url.host() {
        Some(Host::Domain(domain)) if domain.contains('.') && !name_blocked(domain) => Ok(url),
        Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => Ok(url),
        _ => Err("that link cannot be opened".into()),
    }
}

fn name_blocked(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
}

fn destination_public(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip_is_public(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => ip_is_public(IpAddr::V6(ip)),
        Some(Host::Domain(domain)) => !name_blocked(domain) && resolves_public(domain),
        None => false,
    }
}

fn resolves_public(host: &str) -> bool {
    let host = host.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let ok = (host.as_str(), 443)
            .to_socket_addrs()
            .map(|addrs| {
                let addrs: Vec<_> = addrs.collect();
                !addrs.is_empty() && addrs.iter().all(|addr| ip_is_public(addr.ip()))
            })
            .unwrap_or(false);
        let _ = tx.send(ok);
    });
    rx.recv_timeout(Duration::from_secs(4)).unwrap_or(false)
}

fn ip_is_public(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        other => other,
    };
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (b & 0b1100_0000) == 64))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

fn fetch(start: Url) -> Card {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(8)).redirects(0).build();
    let mut current = start;
    for _ in 0..4 {
        if !destination_public(&current) {
            return site_card(&current);
        }
        let response = agent
            .get(current.as_str())
            .set(
                "user-agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .set("accept", "text/html,application/xhtml+xml;q=0.9,image/*;q=0.8,*/*;q=0.5")
            .call();
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) if (300..400).contains(&code) => {
                let Some(next) = redirect_target(&current, response.header("location").unwrap_or("")) else {
                    return site_card(&current);
                };
                current = next;
                continue;
            }
            Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return site_card(&current),
        };
        if !(200..300).contains(&response.status()) {
            return site_card(&current);
        }
        let mime = response.header("content-type").unwrap_or("").to_ascii_lowercase();
        if mime.starts_with("image/") {
            let mut card = site_card(&current);
            card.image = current.to_string();
            card.url = current.to_string();
            return card;
        }
        if !mime.is_empty() && !mime.contains("html") && !mime.contains("xml") && !mime.starts_with("text/") {
            return site_card(&current);
        }
        let mut reader = response.into_reader().take(PAGE_LIMIT as u64);
        let mut bytes = Vec::new();
        if reader.read_to_end(&mut bytes).is_err() && bytes.is_empty() {
            return site_card(&current);
        }
        let html = String::from_utf8_lossy(&bytes);
        return parse_card(&current, &html);
    }
    site_card(&current)
}

fn redirect_target(base: &Url, location: &str) -> Option<Url> {
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    let next = base.join(location).ok()?;
    if next.scheme() != "https" || !next.username().is_empty() || next.password().is_some() {
        return None;
    }
    Some(next)
}

fn site_card(url: &Url) -> Card {
    Card {
        url: url.to_string(),
        title: String::new(),
        description: String::new(),
        image: String::new(),
        site: url.host_str().unwrap_or("").to_string(),
    }
}

fn parse_card(page: &Url, html: &str) -> Card {
    let mut card = site_card(page);
    let mut og_title = String::new();
    let mut twitter_title = String::new();
    let mut og_description = String::new();
    let mut twitter_description = String::new();
    let mut named_description = String::new();
    let mut secure_image = String::new();
    let mut og_image = String::new();
    let mut twitter_image = String::new();
    let mut site_name = String::new();
    let mut search = 0;
    let lower = html.to_ascii_lowercase();
    while let Some(rel) = lower[search..].find("<meta") {
        let start = search + rel;
        let Some(end) = find_tag_end(html, start) else { break };
        let attrs = tag_attrs(&html[start..end]);
        let key = attrs.get("property").or_else(|| attrs.get("name")).map(|value| value.to_ascii_lowercase()).unwrap_or_default();
        let content = collapse(attrs.get("content").map(String::as_str).unwrap_or(""));
        if !content.is_empty() {
            match key.as_str() {
                "og:title" if og_title.is_empty() => og_title = content,
                "twitter:title" if twitter_title.is_empty() => twitter_title = content,
                "og:description" if og_description.is_empty() => og_description = content,
                "twitter:description" if twitter_description.is_empty() => twitter_description = content,
                "description" if named_description.is_empty() => named_description = content,
                "og:image:secure_url" if secure_image.is_empty() => secure_image = content,
                "og:image" if og_image.is_empty() => og_image = content,
                "twitter:image" if twitter_image.is_empty() => twitter_image = content,
                "og:site_name" if site_name.is_empty() => site_name = content,
                _ => {}
            }
        }
        search = end + 1;
    }
    let title_tag = document_title(html);
    card.title = clip(&first_present([og_title.as_str(), twitter_title.as_str(), title_tag.as_str()]), 140);
    card.description = clip(
        &first_present([og_description.as_str(), twitter_description.as_str(), named_description.as_str()]),
        160,
    );
    card.image = https_image(page, &first_present([secure_image.as_str(), og_image.as_str(), twitter_image.as_str()]));
    if !site_name.is_empty() {
        card.site = clip(&site_name, 80);
    }
    card
}

fn first_present(values: [&str; 3]) -> String {
    values.into_iter().find(|value| !value.is_empty()).unwrap_or("").to_string()
}

fn document_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else { return String::new() };
    let Some(open) = html[start..].find('>') else { return String::new() };
    let body = start + open + 1;
    let Some(rel) = lower[body..].find("</title") else { return String::new() };
    collapse(&decode_entities(&html[body..body + rel]))
}

fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, ch) in html[start..].char_indices() {
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '>' => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn tag_attrs(tag: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let mut rest = tag;
    while let Some(start) = rest.find(|ch: char| ch.is_ascii_alphabetic()) {
        rest = &rest[start..];
        let name_len = rest.find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != ':').unwrap_or(rest.len());
        let name = rest[..name_len].to_ascii_lowercase();
        rest = &rest[name_len..];
        rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        rest = rest[1..].trim_start();
        let (value, next) = if rest.starts_with('"') || rest.starts_with('\'') {
            let quote = rest.chars().next().unwrap();
            let body = &rest[1..];
            let end = body.find(quote).unwrap_or(body.len());
            (decode_entities(&body[..end]), body.get(end + 1..).unwrap_or(""))
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (decode_entities(&rest[..end]), &rest[end..])
        };
        if !name.is_empty() && !attrs.contains_key(&name) {
            attrs.insert(name, value);
        }
        rest = next;
    }
    attrs
}

fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(mark) = rest.find('&') {
        out.push_str(&rest[..mark]);
        rest = &rest[mark..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let token = &rest[1..end];
        let decoded = if let Some(hex) = token.strip_prefix("#x").or_else(|| token.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(num) = token.strip_prefix('#') {
            num.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match token {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => None,
            }
        };
        if let Some(ch) = decoded {
            out.push(ch);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() >= max {
            break;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn https_image(page: &Url, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let Ok(url) = page.join(raw) else { return String::new() };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return String::new();
    }
    let allowed = match url.host() {
        Some(Host::Ipv4(ip)) => ip_is_public(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => ip_is_public(IpAddr::V6(ip)),
        Some(Host::Domain(domain)) => domain.contains('.') && !name_blocked(domain),
        None => false,
    };
    if !allowed {
        return String::new();
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::{decode_entities, ip_is_public, name_blocked, parse_card, parse_https};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn https_links_skip_userinfo_and_local_names() {
        assert!(parse_https("https://example.com/a?b=1").is_ok());
        assert!(parse_https("http://example.com").is_err());
        assert!(parse_https("https://user:pass@example.com").is_err());
        assert!(parse_https("https://localhost/admin").is_err());
        assert!(parse_https("https://printer.local/status").is_err());
        assert!(parse_https("https://example.com/a\nb").is_err());
        assert!(name_blocked("metadata.google.internal"));
    }

    #[test]
    fn private_addresses_are_not_previewed() {
        assert!(!ip_is_public(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(!ip_is_public(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
        assert!(!ip_is_public(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9))));
        assert!(!ip_is_public(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(!ip_is_public(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(ip_is_public(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!ip_is_public(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!ip_is_public("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn preview_reads_open_graph_and_title() {
        let page = parse_https("https://example.com/post").unwrap();
        let card = parse_card(
            &page,
            r#"<html><head>
                <title>Fallback &amp; Co</title>
                <meta name="description" content="plain">
                <meta content="A &amp; B" property="og:title">
                <meta property="og:description" content="Hello there">
                <meta property="og:image" content="/cover.jpg">
                <meta property="og:site_name" content="Example">
            </head></html>"#,
        );
        assert_eq!(card.title, "A & B");
        assert_eq!(card.description, "Hello there");
        assert_eq!(card.image, "https://example.com/cover.jpg");
        assert_eq!(card.site, "Example");
        assert_eq!(decode_entities("&#39;ok&#x27;"), "'ok'");
    }

    #[test]
    fn browser_hint_keeps_a_desktop_id() {
        assert_eq!(super::desktop_hint("brave-browser.desktop\n"), "brave-browser");
        assert_eq!(super::desktop_hint("Firefox.desktop"), "firefox");
        assert_eq!(super::desktop_hint("not a browser.desktop"), "");
        assert_eq!(super::script_id("(43,)"), Some(43));
        assert_eq!(super::script_id("43"), None);
    }

    #[test]
    fn missing_open_graph_uses_the_document_title() {
        let page = parse_https("https://example.com/post").unwrap();
        let card = parse_card(&page, "<title>  Line   notes </title>");
        assert_eq!(card.title, "Line notes");
        assert!(card.image.is_empty());
    }
}
