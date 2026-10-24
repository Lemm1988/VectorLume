//! Übertragungswege zur Steuerung: serielle Schnittstelle (USB), Netzwerk (TCP/Telnet) und Simulator.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub trait Transport: Send {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
    /// Liest verfügbare Bytes; `Ok(0)`, wenn gerade nichts anliegt.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialTransport {
    pub fn open(name: &str, baud: u32) -> io::Result<Self> {
        let port = serialport::new(name, baud)
            .timeout(Duration::from_millis(5))
            .open()
            .map_err(|e| io::Error::other(format!("{name}: {e}")))?;
        Ok(Self { port })
    }
}

impl Transport for SerialTransport {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.port, data)?;
        io::Write::flush(&mut self.port)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match io::Read::read(&mut self.port, buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }
}

/// Rohe TCP-Verbindung, wie sie FluidNC, grblHAL oder ESP3D als „Telnet“ anbieten (Standard-Port 23).
pub struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    pub fn connect(address: &str) -> io::Result<Self> {
        let addr = with_default_port(address);
        let mut last = io::Error::other(format!("{addr}: Adresse nicht gefunden"));
        for sa in addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_secs(3)) {
                Ok(stream) => {
                    stream.set_nodelay(true)?;
                    stream.set_read_timeout(Some(Duration::from_millis(5)))?;
                    return Ok(Self { stream });
                }
                Err(e) => last = io::Error::other(format!("{addr}: {e}")),
            }
        }
        Err(last)
    }
}

/// Ergänzt Port 23, falls die Adresse keinen enthält.
pub fn with_default_port(address: &str) -> String {
    let a = address.trim();
    let has_port = match a.rsplit_once(':') {
        // IPv6 ohne Klammern enthält mehrere Doppelpunkte und keinen Port
        Some((host, port)) => {
            !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !host.contains(':')
                || a.starts_with('[') && a.contains("]:")
        }
        None => false,
    };
    if has_port { a.to_string() } else { format!("{a}:23") }
}

impl Transport for TcpTransport {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut self.stream, data)?;
        io::Write::flush(&mut self.stream)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match io::Read::read(&mut self.stream, buf) {
            Ok(0) => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Gegenstelle hat die Verbindung beendet")),
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::TimedOut || e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

pub const SIMULATOR_PORT: &str = "Simulator";
/// Eintrag in der Anschlussliste für eine Netzwerkverbindung; die Adresse folgt als `tcp:host:port`.
pub const NETWORK_PORT: &str = "Netzwerk";
pub const TCP_PREFIX: &str = "tcp:";

/// Verfügbare serielle Anschlüsse plus der eingebaute Simulator.
pub fn available_ports() -> Vec<PortInfo> {
    let mut ports: Vec<PortInfo> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let description = match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => {
                    let product = u.product.clone().unwrap_or_else(|| "USB".into());
                    format!("{product} ({:04X}:{:04X})", u.vid, u.pid)
                }
                serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                serialport::SerialPortType::PciPort => "PCI".into(),
                serialport::SerialPortType::Unknown => String::new(),
            };
            PortInfo { name: p.port_name, description }
        })
        .collect();
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    ports.push(PortInfo { name: NETWORK_PORT.into(), description: "WLAN/LAN, z. B. FluidNC".into() });
    ports.push(PortInfo { name: SIMULATOR_PORT.into(), description: "Ohne Laser testen".into() });
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port() {
        assert_eq!(with_default_port(" 192.168.4.1 "), "192.168.4.1:23");
        assert_eq!(with_default_port("laser.local:8080"), "laser.local:8080");
        assert_eq!(with_default_port("[fe80::1]:23"), "[fe80::1]:23");
    }
}
