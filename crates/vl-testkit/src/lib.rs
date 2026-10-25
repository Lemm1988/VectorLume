//! Gemeinsame Testhilfen der VectorLume-Crates (nur für Tests, wird nicht ausgeliefert).
//!
//! Robustheit: [`check`] wandelt kleine, gültige Beispieldateien nach einem festen Zufallsmuster ab (abschneiden,
//! Bytes verfälschen, Zahlen durch Extremwerte ersetzen, Abschnitte entfernen oder vervielfachen) und meldet jeden
//! Absturz, jede zu lange Rechenzeit und jedes Hängen. Fehlerhafte Eingaben werden zum Nachvollziehen im Temp-Ordner
//! abgelegt; mit `VECTORLUME_ROBUST_TRACE=1` wird jeder Fall vor dem Aufruf ausgegeben (für Abbrüche, die der Test
//! nicht abfangen kann, etwa fehlender Speicher).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Ab dieser Rechenzeit für eine einzelne kleine Datei gilt ein Import als Fehler (Testbuild ohne Optimierung).
const SLOW: Duration = Duration::from_secs(5);
/// Danach gilt der Import als hängend.
const HANG: Duration = Duration::from_secs(30);

/// Werte, die anstelle von Zahlen eingesetzt werden.
const EXTREMES: [&str; 11] =
    ["1e308", "-1e308", "1e30", "NaN", "inf", "0", "-1", "4294967296", "99999999", "0.000000001", "-0"];

/// Fester Zufallsgenerator (xorshift64*), damit jeder Lauf dieselben Fälle prüft.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Bereiche der Zahlen im Text (mit Vorzeichen und Nachkommastellen).
fn number_spans(data: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let starts =
            data[i].is_ascii_digit() && (i == 0 || !(data[i - 1].is_ascii_alphanumeric() || data[i - 1] == b'.'));
        if !starts {
            i += 1;
            continue;
        }
        let start = if i > 0 && data[i - 1] == b'-' { i - 1 } else { i };
        while i < data.len() && (data[i].is_ascii_digit() || data[i] == b'.') {
            i += 1;
        }
        spans.push((start, i));
    }
    spans
}

fn replace_spans(data: &[u8], spans: &[(usize, usize)], with: &dyn Fn(usize) -> Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut last = 0;
    for (k, &(a, b)) in spans.iter().enumerate() {
        out.extend_from_slice(&data[last..a]);
        out.extend(with(k));
        last = b;
    }
    out.extend_from_slice(&data[last..]);
    out
}

/// Alle Abwandlungen einer Beispieldatei mit Beschreibung.
pub fn variants(sample: &[u8], seed: u64) -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let len = sample.len();
    let mut out = vec![("unverändert".to_string(), sample.to_vec())];
    for k in 0..20 {
        let n = len * k / 20;
        out.push((format!("abgeschnitten nach {n} Bytes"), sample[..n].to_vec()));
    }
    out.push(("letztes Byte fehlt".into(), sample[..len.saturating_sub(1)].to_vec()));
    const BYTES: [u8; 12] = [0, 0xFF, b'<', b'>', b'"', b'-', b'9', b' ', b'\n', b'%', b'*', b'/'];
    for _ in 0..200 {
        let mut d = sample.to_vec();
        let mut what = Vec::new();
        for _ in 0..1 + rng.below(4) {
            let i = rng.below(len);
            let b = if rng.below(3) == 0 { rng.next() as u8 } else { BYTES[rng.below(BYTES.len())] };
            d[i] = b;
            what.push(format!("{i}={b:#04x}"));
        }
        out.push((format!("Bytes {}", what.join(",")), d));
    }
    let spans = number_spans(sample);
    if !spans.is_empty() {
        for x in EXTREMES {
            out.push((format!("alle Zahlen = {x}"), replace_spans(sample, &spans, &|_| x.as_bytes().to_vec())));
        }
        for _ in 0..200 {
            let k = rng.below(spans.len());
            let x = EXTREMES[rng.below(EXTREMES.len())];
            let (a, b) = spans[k];
            let old = String::from_utf8_lossy(&sample[a..b]).into_owned();
            let d = replace_spans(sample, &spans[k..=k], &|_| x.as_bytes().to_vec());
            out.push((format!("Zahl „{old}“ an {a} = {x}"), d));
        }
    }
    for _ in 0..50 {
        let a = rng.below(len);
        let b = (a + 1 + rng.below(len / 4 + 1)).min(len);
        let mut d = sample[..a].to_vec();
        d.extend_from_slice(&sample[b..]);
        out.push((format!("Bytes {a}..{b} entfernt"), d));
    }
    for _ in 0..20 {
        let a = rng.below(len);
        let b = (a + 1 + rng.below(len / 4 + 1)).min(len);
        let times = 1 + rng.below(20);
        let mut d = sample[..b].to_vec();
        for _ in 0..times {
            d.extend_from_slice(&sample[a..b]);
        }
        d.extend_from_slice(&sample[b..]);
        out.push((format!("Bytes {a}..{b} {times}× wiederholt"), d));
    }
    out
}

/// Führt `import` für alle Abwandlungen aller Beispiele aus und meldet Abstürze, langsame und hängende Fälle.
/// `import` gibt zurück, ob die Datei gelesen wurde; die unveränderten Beispiele müssen das.
pub fn check(format: &'static str, samples: Vec<Vec<u8>>, import: fn(&[u8]) -> bool) {
    let cases: Vec<(String, Vec<u8>)> = samples
        .iter()
        .enumerate()
        .flat_map(|(s, sample)| {
            variants(sample, s as u64 + 1)
                .into_iter()
                .map(move |(what, d)| (format!("{format} Beispiel {s}: {what}"), d))
        })
        .collect();
    for (s, sample) in samples.iter().enumerate() {
        assert!(import(sample), "{format} Beispiel {s} lässt sich nicht importieren");
    }
    let inputs: Vec<Vec<u8>> = cases.iter().map(|c| c.1.clone()).collect();
    let labels: Vec<String> = cases.iter().map(|c| c.0.clone()).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("robust-{format}"))
        .stack_size(16 << 20)
        .spawn(move || {
            let trace = std::env::var_os("VECTORLUME_ROBUST_TRACE").is_some();
            for (i, (label, data)) in cases.into_iter().enumerate() {
                if trace {
                    eprintln!("{i}: {label}");
                }
                let start = Instant::now();
                let ok = catch_unwind(AssertUnwindSafe(|| import(&data))).is_ok();
                if tx.send((i, ok, start.elapsed())).is_err() {
                    return;
                }
            }
        })
        .unwrap();
    let mut failures = Vec::new();
    let mut fail = |i: usize, why: String| {
        let file = std::env::temp_dir().join(format!("vectorlume-robust-{format}-{i}.bin"));
        let _ = std::fs::write(&file, &inputs[i]);
        failures.push(format!("{}: {why} ({})", labels[i], file.display()));
    };
    let mut next = 0;
    while next < labels.len() {
        match rx.recv_timeout(HANG) {
            Ok((i, ok, t)) => {
                if !ok {
                    fail(i, "Absturz".into());
                } else if t > SLOW {
                    fail(i, format!("zu langsam ({:.1} s)", t.as_secs_f64()));
                }
                next = i + 1;
            }
            Err(_) => {
                fail(next, "hängt".into());
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{} von {} Fällen fehlerhaft:\n{}", failures.len(), labels.len(), failures.join("\n"));
}
