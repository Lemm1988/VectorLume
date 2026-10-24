//! Entwicklerwerkzeug: zeigt, was der Import aus einer Datei liest.
//! `cargo run -p vl-io --example import_info -- <datei>`

use vl_core::Affine;
use vl_io::ImportedItem;

fn main() {
    for arg in std::env::args().skip(1) {
        let start = std::time::Instant::now();
        match vl_io::import_file(std::path::Path::new(&arg), None) {
            Ok(imp) => {
                let mut segs = 0;
                let mut bounds: Option<vl_core::Rect> = None;
                for item in &imp.items {
                    if let ImportedItem::Path { path, .. } = item {
                        segs += path.segment_count();
                        let b = path.bounds(&Affine::IDENTITY);
                        bounds = Some(bounds.map_or(b, |a| a.union(&b)));
                    }
                }
                println!(
                    "{arg}: {} Objekte, {segs} Segmente, Umriss {bounds:?}, {:?}",
                    imp.items.len(),
                    start.elapsed()
                );
                for w in imp.warnings {
                    println!("  Hinweis: {w}");
                }
            }
            Err(e) => println!("{arg}: Fehler: {e}"),
        }
    }
}
