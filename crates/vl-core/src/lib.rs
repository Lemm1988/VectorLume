//! Kernbibliothek von VectorLume: Geometrie, Pfade, Ebenen, Dokument und Projektdateien.

pub mod document;
pub mod geom;
pub mod layer;
pub mod material;
pub mod nodes;
pub mod ops;
pub mod path;
pub mod project;
pub mod text;

pub use document::{Document, GroupId, ImageAdjust, ImageObject, Object, ObjectId, RasterImage, Shape};
pub use geom::{Affine, Point, Rect};
pub use layer::{CutSettings, DitherMode, LAYER_COUNT, Layer, LayerMode, PALETTE};
pub use material::{MaterialEntry, MaterialLibrary};
pub use path::{PathBuilder, Polyline, Segment, SubPath, VectorPath};
pub use text::{TextAlign, TextData, TextSpec};
