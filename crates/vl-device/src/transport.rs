//! Übertragungswege zur Steuerung: serielle Schnittstelle (USB) und Simulator.

use std::io;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
}

pub const SIMULATOR_PORT: &str = "Simulator";

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
    ports.push(PortInfo { name: SIMULATOR_PORT.into(), description: "Ohne Laser testen".into() });
    ports
}
