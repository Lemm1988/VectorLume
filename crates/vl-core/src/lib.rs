//! Kernbibliothek von VectorLume: Geometrie, Pfade, Ebenen, Dokument und Projektdateien.

pub mod document;
pub mod geom;
pub mod layer;
pub mod material;
pub mod path;
pub mod project;

pub use document::{Document, GroupId, ImageAdjust, ImageObject, Object, ObjectId, RasterImage, Shape};
pub use geom::{Affine, Point, Rect};
pub use layer::{CutSettings, DitherMode, LAYER_COUNT, Layer, LayerMode, PALETTE};
pub use material::{MaterialEntry, MaterialLibrary};
pub use path::{PathBuilder, Polyline, Segment, SubPath, VectorPath};
