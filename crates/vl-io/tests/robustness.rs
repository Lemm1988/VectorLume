//! Robustheit der Importe: abgeschnittene, verfälschte und mit Extremwerten gefüllte Dateien dürfen weder abstürzen
//! noch hängen (siehe `vl_testkit::check`). Jedes Format bekommt eigene kleine Beispieldateien.

use vl_testkit::check;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 60) as u8, (y * 80) as u8, 128, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn png_base64() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(png(4, 3))
}

#[test]
fn svg_survives_damage() {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="80mm" viewBox="0 0 100 80">
 <defs><linearGradient id="g"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs>
 <g transform="translate(5,5) rotate(10) scale(1.5,0.5)">
  <rect x="1" y="2" width="30" height="20" rx="3" fill="none" stroke="red" stroke-width="0.2"/>
  <circle cx="50" cy="40" r="10" fill="url(#g)"/>
  <ellipse cx="70" cy="20" rx="8" ry="4" stroke="black" fill="none"/>
  <path d="M10 60 C 20 50, 40 70, 50 60 S 80 50, 90 60 Q 95 70 90 75 T 85 78 A 5 5 0 1 1 80 75 Z m 2 2 h 5 v 5 l -3 1 z"
        stroke="green" fill="none" stroke-dasharray="2 1"/>
  <polyline points="0,0 10,10 20,0 30,10" stroke="black" fill="none"/>
  <polygon points="40,0 50,10 60,0" fill="black"/>
  <line x1="0" y1="79" x2="99" y2="79" stroke="black"/>
  <use href="#k" x="10" y="10"/>
 </g>
 <path id="k" d="M0 0 L5 5" stroke="black"/>
 <image x="60" y="50" width="20" height="15" href="data:image/png;base64,{}"/>
</svg>"##,
        png_base64()
    );
    check("svg", vec![svg.into_bytes()], |d| vl_io::svg::import_svg(d, None).is_ok());
}

const DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n8\n0\n2\nTEIL\n\
70\n0\n10\n0.0\n20\n0.0\n30\n0.0\n0\nLINE\n8\n0\n10\n0.0\n20\n0.0\n30\n0.0\n11\n5.0\n21\n5.0\n31\n0.0\n0\nINSERT\n8\n0\n\
2\nTEIL\n10\n1.0\n20\n1.0\n30\n0.0\n0\nENDBLK\n8\n0\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0.0\n20\n0.0\n\
30\n0.0\n11\n100.0\n21\n50.0\n31\n0.0\n0\nCIRCLE\n8\n0\n10\n50.0\n20\n50.0\n30\n0.0\n40\n25.0\n0\nARC\n8\n0\n10\n20.0\n\
20\n20.0\n30\n0.0\n40\n10.0\n50\n0.0\n51\n90.0\n0\nLWPOLYLINE\n8\n0\n90\n3\n70\n1\n10\n0.0\n20\n0.0\n42\n0.5\n10\n10.0\n\
20\n0.0\n10\n10.0\n20\n10.0\n0\nSPLINE\n8\n0\n70\n8\n71\n3\n72\n8\n73\n4\n40\n0.0\n40\n0.0\n40\n0.0\n40\n0.0\n40\n1.0\n\
40\n1.0\n40\n1.0\n40\n1.0\n10\n0.0\n20\n0.0\n30\n0.0\n10\n10.0\n20\n20.0\n30\n0.0\n10\n20.0\n20\n-20.0\n30\n0.0\n10\n\
30.0\n20\n0.0\n30\n0.0\n0\nELLIPSE\n8\n0\n10\n0.0\n20\n0.0\n30\n0.0\n11\n20.0\n21\n0.0\n31\n0.0\n40\n0.5\n41\n0.0\n42\n\
6.283185307\n0\nPOLYLINE\n8\n0\n66\n1\n70\n1\n0\nVERTEX\n8\n0\n10\n0.0\n20\n0.0\n42\n1.0\n0\nVERTEX\n8\n0\n10\n10.0\n20\n\
0.0\n0\nVERTEX\n8\n0\n10\n5.0\n20\n8.0\n0\nSEQEND\n8\n0\n0\nINSERT\n8\n0\n2\nTEIL\n10\n50.0\n20\n10.0\n30\n0.0\n41\n2.0\n\
42\n2.0\n50\n45.0\n0\nENDSEC\n0\nEOF\n";

#[test]
fn dxf_survives_damage() {
    check("dxf", vec![DXF.as_bytes().to_vec()], |d| vl_io::dxf::import_dxf(d).is_ok());
}

/// PDF mit unkomprimiertem Inhalt (damit die Zahlen im Zeichenstrom verändert werden) und ein komprimiertes aus
/// dem eigenen PDF-Export.
fn pdf_samples() -> Vec<Vec<u8>> {
    use lopdf::{Object, Stream, dictionary};
    let mut pdf = lopdf::Document::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        vec![0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, 255],
    );
    let image_id = pdf.add_object(image);
    let content = b"q 1 0 0 1 10 10 cm 0.5 w 0 0 m 100 0 l 100 100 l h S 50 50 m 60 50 70 60 80 50 c S \
        10 10 40 20 re f 5 5 m 20 5 25 10 v 30 30 y W n Q q 20 0 0 20 30 30 cm /Im1 Do Q"
        .to_vec();
    let content_id = pdf.add_object(Stream::new(dictionary! {}, content));
    let resources_id = pdf.add_object(dictionary! { "XObject" => dictionary! { "Im1" => image_id } });
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), Object::Real(200.5)],
    });
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }),
    );
    let catalog_id = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog_id);
    let mut plain = Vec::new();
    pdf.save_to(&mut plain).unwrap();

    let mut doc = vl_core::Document::new();
    doc.add_path("Rechteck", 0, vl_core::VectorPath::rect(20.0, 30.0, 40.0, 25.0, 3.0));
    let exported =
        vl_io::pdf_export::export_pdf(&doc, vl_core::Rect::from_xywh(0.0, 0.0, 100.0, 80.0), "Test").unwrap();
    vec![plain, exported]
}

#[test]
fn pdf_survives_damage() {
    check("pdf", pdf_samples(), |d| vl_io::pdf::import_pdf(d).is_ok());
}

#[test]
fn hpgl_survives_damage() {
    let plt = b"IN;SP1;PU0,0;PD100,0,100,100;PU;PA2000,2000;CI200;PR400,0;AA0,0,90;AR10,10,-45;PD;PR10,10,-5,3;\
        PU;LBHallo\x03;SP2;PA0,0;PD;PR0,500;SP0;";
    check("hpgl", vec![plt.to_vec()], |d| vl_io::hpgl::import_hpgl(d).is_ok());
}

const GERBER: &str = "%FSLAX33Y33*%\n%MOMM*%\n%TF.FileFunction,Copper,L1,Top*%\n\
%AMBOX*0 Rechteck mit Loch*$3=$1x2*21,1,$3,$2,0,0,0*1,0,1,0,0*%\n%AMRING*1,1,$1,0,0*1,0,$2,0,0*4,1,3,0,0,1,0,1,1,0,0,0*%\n\
%ADD10C,0.5*%\n%ADD11R,1X2*%\n%ADD12O,1X2*%\n%ADD13P,2X6X15*%\n%ADD20BOX,2X2*%\n%ADD21RING,3X1*%\n\
%SRX2Y3I5.0J5.0*%\nD10*\nX0Y0D02*\nG01X10000Y0D01*\nG75*\nG03X0Y10000I-10000J0D01*\nD11*X20000Y20000D03*\n\
D12*X25000Y20000D03*\nD13*X30000Y20000D03*\nD20*X35000Y20000D03*\nD21*X40000Y20000D03*\n%SR*%\n\
G36*\nX0Y0D02*\nG01X5000Y0D01*\nY5000D01*\nX0D01*\nY0D01*\nG37*\n%LPC*%\nD10*X2500Y2500D03*\n%LPD*%\nM02*\n";

#[test]
fn gerber_survives_damage() {
    check("gerber", vec![GERBER.as_bytes().to_vec()], |d| {
        let copper = vl_io::gerber::import_gerber(d, "kupfer", false).is_ok();
        vl_io::gerber::import_gerber(d, "kontur", true).is_ok() && copper
    });
}

#[test]
fn excellon_survives_damage() {
    let drl = "M48\nMETRIC,TZ\nT1C0.8\nT2C3.0\n%\nT1\nX10.0Y10.0\nX20.0Y10.0G85X30.0Y10.0\nT2\nG00X40.0Y40.0\nM15\n\
        G01X50.0Y40.0\nM16\nM30\n";
    let inch = "M48\nINCH,LZ\nT01C0.0320\n%\nT01\nX001000Y001000\nX002000Y001000\nM30\n";
    check("excellon", vec![drl.as_bytes().to_vec(), inch.as_bytes().to_vec()], |d| {
        vl_io::excellon::import_excellon(d, "bohrungen").is_ok()
    });
}

fn binary_stl() -> Vec<u8> {
    let tris = [
        [[0.0f32, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 5.0]],
        [[0.0, 0.0, 0.0], [10.0, 10.0, 5.0], [0.0, 10.0, 5.0]],
    ];
    let mut out = vec![0u8; 80];
    out.extend((tris.len() as u32).to_le_bytes());
    for t in tris {
        out.extend([0u8; 12]);
        for c in t.iter().flatten() {
            out.extend(c.to_le_bytes());
        }
        out.extend([0u8; 2]);
    }
    out
}

#[test]
fn model3d_survives_damage() {
    let obj = "# Würfelecke\nv 0 0 0\nv 10 0 0\nv 10 10 5\nv 0 10 5\nvt 0 0\nvn 0 0 1\nf 1/1/1 2/1/1 3/1/1\nf 1 3 4\n\
        f -1 -2 -3\nf 1 2 3 4\n";
    let stl = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 10 0 0\nvertex 10 10 5\nendloop\nendfacet\n\
        facet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 10 10 5\nvertex 0 10 5\nendloop\nendfacet\nendsolid t\n";
    check("modell", vec![obj.as_bytes().to_vec(), stl.as_bytes().to_vec(), binary_stl()], |d| {
        use vl_io::model3d::{DepthOptions, View, depth_map, parse_model};
        let Ok(mesh) = parse_model(d) else { return false };
        // alle Ansichten rechnen, auch wenn eine scheitert
        let failed = [View::Top, View::Front, View::Right]
            .into_iter()
            .filter(|&view| {
                depth_map(&mesh, &DepthOptions { view, width_mm: 50.0, px_per_mm: 4.0, invert: false }).is_err()
            })
            .count();
        failed == 0
    });
}

#[test]
fn lightburn_survives_damage() {
    let lbrn = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<LightBurnProject AppVersion="1.0.0" FormatVersion="1" MaterialHeight="3" MirrorX="False" MirrorY="False">
    <CutSetting type="Cut">
        <index Value="0"/><name Value="Kontur"/><minPower Value="10"/><maxPower Value="40"/><speed Value="100"/>
        <numPasses Value="2"/><priority Value="1"/>
    </CutSetting>
    <CutSetting_Img type="Image">
        <index Value="1"/><name Value="Bild"/><maxPower Value="20"/><speed Value="200"/><interval Value="0.1"/>
    </CutSetting_Img>
    <CutSetting type="Scan">
        <index Value="2"/><name Value="Fläche"/><maxPower Value="30"/><speed Value="300"/><interval Value="0.08"/>
        <crossHatch Value="1"/><angle Value="45"/>
    </CutSetting>
    <Shape Type="Rect" CutIndex="0" W="20" H="10" Cr="2"><XForm>1 0 0 1 30 40</XForm></Shape>
    <Shape Type="Ellipse" CutIndex="2" Rx="5" Ry="3"><XForm>0.7 0.7 -0.7 0.7 100 100</XForm></Shape>
    <Shape Type="Group"><XForm>1 0 0 1 10 0</XForm><Children>
        <Shape Type="Path" CutIndex="0" VertID="0" PrimID="0"><XForm>1 0 0 1 0 0</XForm>
            <VertList>V0 0c0x1c1x1V10 0c0x10c0y5c1x1V10 10c0x1c1x10c1y7</VertList><PrimList>L0 1B1 2L2 0</PrimList>
        </Shape>
        <Shape Type="Path" CutIndex="0" VertID="0" PrimID="0"><XForm>1 0 0 1 50 0</XForm></Shape>
    </Children></Shape>
    <Shape Type="Path" CutIndex="0"><XForm>1 0 0 1 0 0</XForm>
        <VertList>V0 0c0x1c1x1V5 5c0x1c1x1V9 1c0x1c1x1</VertList><PrimList>LineClosed</PrimList>
    </Shape>
    <Shape Type="Bitmap" CutIndex="1" W="20" H="15" File="bild.png" Data="{}"><XForm>1 0 0 1 60 60</XForm></Shape>
    <Notes ShowOnLoad="0" Notes="Probe"/>
</LightBurnProject>"#,
        png_base64()
    );
    check("lightburn", vec![lbrn.into_bytes()], |d| vl_io::import_lightburn(d, 400.0, None).is_ok());
}

#[test]
fn csv_survives_damage() {
    let csv = "Name;Nummer;Datum\nAnna;1;2024-01-01\n\"Bob; Jr\";2;\"x\"\"y\"\n\nÄrger;3,5;\n";
    let tsv = "\u{feff}a\tb\n1\t2\n";
    check("csv", vec![csv.as_bytes().to_vec(), tsv.as_bytes().to_vec()], |d| vl_io::parse_csv(d, "daten.csv").is_ok());
}

#[test]
fn images_survive_damage() {
    let mut bmp = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255])))
        .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
        .unwrap();
    check("bild", vec![png(4, 3), bmp], |d| vl_io::image::import_image(d, "bild.png").is_ok());
}
