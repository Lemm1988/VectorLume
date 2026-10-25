//! Kernbibliothek von VectorLume: Geometrie, Pfade, Ebenen, Dokument und Projektdateien.

pub mod cleanup;
pub mod document;
pub mod firmware;
pub mod geom;
pub mod layer;
pub mod material;
pub mod nest;
pub mod nodes;
pub mod ops;
pub mod path;
pub mod project;
pub mod text;
pub mod trim;

pub use document::{Document, GroupId, ImageAdjust, ImageObject, Object, ObjectId, ProjectNotes, RasterImage, Shape};
pub use firmware::Firmware;
pub use geom::{Affine, Point, Rect};
pub use layer::{CutSettings, DitherMode, FillGrouping, LAYER_COUNT, Layer, LayerMode, PALETTE};
pub use material::{MaterialEntry, MaterialLibrary};
pub use path::{PathBuilder, Polyline, Segment, SubPath, VectorPath};
pub use text::{BarcodeKind, BarcodeSpec, DataList, QrLevel, TextAlign, TextData, TextSpec};
