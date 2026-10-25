//! Kernbibliothek von VectorLume: Geometrie, Pfade, Ebenen, Dokument und Projektdateien.

pub mod boxgen;
pub mod cleanup;
pub mod corners;
pub mod dock;
pub mod document;
pub mod firmware;
pub mod geom;
pub mod hinge;
pub mod layer;
pub mod material;
pub mod nest;
pub mod nodes;
pub mod ops;
pub mod path;
pub mod primitive;
pub mod project;
pub mod slots;
pub mod stencil;
pub mod text;
pub mod trim;
pub mod warp;

pub use document::{
    Document, GroupId, ImageAdjust, ImageObject, Object, ObjectId, ProjectNotes, RasterImage, Shape, ZOrder,
};
pub use firmware::Firmware;
pub use geom::{Affine, Point, Rect};
pub use layer::{CutSettings, DitherMode, FillGrouping, LAYER_COUNT, Layer, LayerMode, PALETTE};
pub use material::{MaterialEntry, MaterialLibrary};
pub use path::{PathBuilder, Polyline, Segment, SubPath, VectorPath};
pub use text::{BarcodeKind, BarcodeSpec, DataList, QrLevel, TextAlign, TextData, TextSpec};
