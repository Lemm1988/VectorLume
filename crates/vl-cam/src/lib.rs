//! Werkzeugwege, Optimierung, Laufzeitschätzung und G-Code für VectorLume.

pub mod estimate;
pub mod gcode;
pub mod job;
pub mod machine;
pub mod optimize;
pub mod testpattern;
pub mod toolpath;

pub use estimate::{Estimate, format_duration};
pub use gcode::{CoordMode, strip_for_streaming};
pub use job::{Job, OutputMapping, build_job, finish_job, frame_gcode, placement};
pub use machine::{Anchor, JobSettings, MachineProfile, OriginCorner, Rotary, RotaryAxis, RotaryKind, StartFrom};
pub use testpattern::{TestAxis, TestLayout, TestParam, TestPattern};
pub use toolpath::{Block, BlockKind, Op, RasterPreview, Toolpath};
