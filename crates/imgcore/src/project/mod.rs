pub mod jobs;
pub mod output;
pub mod preset;
pub mod scan;
pub mod split;
pub mod svg_export;

pub use jobs::{run_file_job, FileJob, JobResult};
pub use output::{plan_output_path, resolve_conflict, write_atomic};
pub use preset::{ConflictPolicy, OutputMode, OutputPolicy, PresetDefaults, ProjectPreset, ScanPolicy, SpecPolicy, PRESET_FILE};
pub use scan::{is_image_path, scan, walk_images, ScanEntry, ScanOptions, ScanReport};
pub use split::{default_out_dir, sanitize_name, split_file, SplitFile, SplitJob, SplitResult};
pub use svg_export::{export_icon_set, export_svg, IconSetItem, SvgExportFile, SvgExportJob, SvgExportResult};
