//! Netzwerkkamera: Einzelbild über HTTP holen – Schnappschuss-URL (JPEG/PNG, z. B. `http://192.168.1.50/capture`
//! einer ESP32-CAM) oder MJPEG-Strom, aus dem das erste vollständige Bild genommen wird. Bewusst ohne
//! HTTP-Bibliothek: nur `http://`, optional mit `benutzer:passwort@` (Basic-Anmeldung).

use base64::Engine;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Größere Antworten werden abgebrochen (Schutz vor endlosen Strömen ohne Bildende).
const MAX_BYTES: usize = 30 * 1024 * 1024;

struct Url<'a> {
    host: &'a str,
    port: u16,
    path: &'a str,
    auth: Option<&'a str>,
}

fn parse(url: &str) -> Result<Url<'_>, String> {
    let url = url.trim();
    if url.starts_with("https://") {
        return Err("HTTPS wird nicht unterstützt – Kameras im eigenen Netz sind meist über http:// erreichbar.".into());
    }
    let rest = url.strip_prefix("http://").ok_or("Die Adresse muss mit http:// beginnen.")?;
    let (authority, path) = rest.find('/').map_or((rest, "/"), |i| (&rest[..i], &rest[i..]));
    let (auth, hostport) = match authority.rsplit_once('@') {
        Some((a, h)) => (Some(a), h),
        None => (None, authority),
    };
    let port = |p: &str| p.parse::<u16>().map_err(|_| format!("Ungültiger Port „{p}“."));
    // IPv6-Adressen stehen in eckigen Klammern: [::1]:8080
    let (host, port) = match hostport.strip_prefix('[') {
        Some(v6) => {
            let (h, after) = v6.split_once(']').ok_or("In der Adresse fehlt die schließende Klammer.")?;
            (h, after.strip_prefix(':').map_or(Ok(80), port)?)
        }
        None => match hostport.rsplit_once(':') {
            Some((h, p)) => (h, port(p)?),
            None => (hostport, 80),
        },
    };
    if host.is_empty() {
        return Err("In der Adresse fehlt der Rechnername.".into());
    }
    Ok(Url { host, port, path, auth })
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Holt ein Bild (Rohdaten der Datei, z. B. JPEG) von `url`.
pub fn fetch_image(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let u = parse(url)?;
    // Tests sprechen nur lokale Testserver an
    if cfg!(test) && !(u.host.starts_with("127.") || u.host == "localhost") {
        return Err(format!("Im Test keine Verbindung zu {}", u.host));
    }
    let addr = (u.host, u.port)
        .to_socket_addrs()
        .map_err(|e| format!("Kamera „{}“ nicht gefunden: {e}", u.host))?
        .next()
        .ok_or_else(|| format!("Kamera „{}“ nicht gefunden.", u.host))?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("Keine Verbindung zur Kamera: {e}"))?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    // HTTP/1.0: ohne „chunked“-Kodierung, Verbindung endet nach der Antwort
    let host = if u.host.contains(':') { format!("[{}]", u.host) } else { u.host.to_string() };
    let mut req = format!("GET {} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: VectorLume\r\nAccept: image/*\r\n", u.path);
    if let Some(a) = u.auth {
        req.push_str(&format!("Authorization: Basic {}\r\n", base64::engine::general_purpose::STANDARD.encode(a)));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).map_err(|e| format!("Keine Verbindung zur Kamera: {e}"))?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut header_end = None;
    let mut multipart = false;
    let mut jpeg_start = None;
    loop {
        let n = match s.read(&mut chunk) {
            Ok(n) => n,
            Err(e) if buf.is_empty() => return Err(format!("Die Kamera antwortet nicht: {e}")),
            // Zeitüberschreitung mitten im Strom: mit dem Vorhandenen weiterarbeiten
            Err(_) => 0,
        };
        // Gesucht wird nur im neuen Teil (plus Überlappung für Marken auf der Grenze) – sonst würde ein Strom ohne
        // Bildende den Puffer bis zur Obergrenze immer wieder ganz durchsuchen (quadratischer Aufwand).
        let old = buf.len();
        buf.extend_from_slice(&chunk[..n]);
        if header_end.is_none() {
            header_end = find(&buf, b"\r\n\r\n", old.saturating_sub(3)).map(|i| i + 4);
            if let Some(end) = header_end {
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let status = head.split_whitespace().nth(1).unwrap_or("");
                if status != "200" {
                    let line = head.lines().next().unwrap_or("").trim().to_string();
                    return Err(format!("Die Kamera meldet „{line}“."));
                }
                multipart = head.to_ascii_lowercase().contains("multipart/");
            }
        }
        // MJPEG-Strom: sobald ein Bild vollständig ist, aufhören
        if let Some(end) = header_end
            && multipart
        {
            if jpeg_start.is_none() {
                jpeg_start = find(&buf, &[0xFF, 0xD8], end.max(old.saturating_sub(1)));
            }
            if let Some(start) = jpeg_start
                && let Some(stop) = find(&buf, &[0xFF, 0xD9], (start + 2).max(old.saturating_sub(1)))
            {
                return Ok(buf[start..stop + 2].to_vec());
            }
        }
        if n == 0 {
            break;
        }
        if buf.len() > MAX_BYTES {
            return Err("Die Kamera liefert kein vollständiges Bild.".into());
        }
    }
    let end = header_end.ok_or("Die Kamera hat keine gültige HTTP-Antwort geschickt.")?;
    let body = buf[end..].to_vec();
    if body.is_empty() {
        return Err("Die Kamera hat ein leeres Bild geschickt.".into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 6, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    /// Einmal-Server: beantwortet eine Anfrage mit `head` + `body` und gibt die Anfrage zurück.
    fn serve(head: String, body: Vec<u8>, keep_open: bool) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let t = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut req = vec![0u8; 4096];
            let n = c.read(&mut req).unwrap();
            // der Client darf vorzeitig auflegen
            let _ = c.write_all(head.as_bytes());
            let _ = c.write_all(&body);
            if keep_open {
                // Strom läuft weiter, bis der Client auflegt
                std::thread::sleep(Duration::from_millis(1500));
            }
            String::from_utf8_lossy(&req[..n]).to_string()
        });
        (url, t)
    }

    #[test]
    fn snapshot_with_basic_auth() {
        let img = jpeg();
        let (url, t) = serve("HTTP/1.0 200 OK\r\nContent-Type: image/jpeg\r\n\r\n".into(), img.clone(), false);
        let url = url.replace("http://", "http://admin:geheim@") + "/capture?x=1";
        let got = fetch_image(&url, Duration::from_secs(5)).unwrap();
        assert_eq!(got, img);
        let req = t.join().unwrap();
        assert!(req.starts_with("GET /capture?x=1 HTTP/1.0\r\n"), "{req}");
        assert!(req.contains("Authorization: Basic YWRtaW46Z2VoZWlt\r\n"), "{req}");
        assert!(image::load_from_memory(&got).is_ok());
    }

    #[test]
    fn first_frame_of_mjpeg_stream() {
        let img = jpeg();
        let mut body = b"--frame\r\nContent-Type: image/jpeg\r\n\r\n".to_vec();
        body.extend(&img);
        body.extend(b"\r\n--frame\r\nContent-Type: image/jpeg\r\n\r\n");
        body.extend(&img[..10]);
        let head = "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\n\r\n".to_string();
        let (url, t) = serve(head, body, true);
        let started = std::time::Instant::now();
        let got = fetch_image(&(url + "/stream"), Duration::from_secs(5)).unwrap();
        assert_eq!(got, img);
        assert!(started.elapsed() < Duration::from_secs(1), "nicht auf das Ende des Stroms warten");
        t.join().unwrap();
    }

    /// Ströme ohne Kopfende oder ohne Bildende enden an der Obergrenze – schnell, ohne den Puffer immer wieder
    /// ganz zu durchsuchen.
    #[test]
    fn endless_streams_are_cut_off_quickly() {
        let mjpeg = "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\n\r\n".to_string();
        let mut frame = b"--frame\r\n\r\n\xFF\xD8".to_vec();
        frame.resize(MAX_BYTES + 64 * 1024, 0x55);
        let no_header = vec![b'x'; MAX_BYTES + 64 * 1024];
        for (head, body) in [(mjpeg, frame), ("HTTP/1.0 200 OK\r\n".to_string(), no_header)] {
            let (url, t) = serve(head, body, false);
            let started = std::time::Instant::now();
            let e = fetch_image(&url, Duration::from_secs(5)).unwrap_err();
            assert!(e.contains("kein vollständiges Bild"), "{e}");
            assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
            t.join().unwrap();
        }
    }

    #[test]
    fn errors() {
        let (url, t) = serve("HTTP/1.0 404 Not Found\r\n\r\n".into(), Vec::new(), false);
        let e = fetch_image(&url, Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("404"), "{e}");
        t.join().unwrap();
        assert!(fetch_image("https://kamera/bild.jpg", Duration::from_secs(1)).unwrap_err().contains("HTTPS"));
        assert!(fetch_image("ftp://kamera", Duration::from_secs(1)).is_err());
        assert!(fetch_image("http://127.0.0.1:x/", Duration::from_secs(1)).unwrap_err().contains("Port"));
        let p = parse("http://[::1]:8080/a").unwrap();
        assert_eq!((p.host, p.port, p.path), ("::1", 8080, "/a"));
        assert_eq!(parse("http://[fe80::2]/").unwrap().port, 80);
        let p = parse("http://cam.local").unwrap();
        assert_eq!((p.host, p.port, p.path, p.auth), ("cam.local", 80, "/", None));
    }
}
