pub mod jobs;
pub mod output;
pub mod preset;
pub mod scan;
pub mod split;

pub use jobs::{run_file_job, FileJob, JobResult};
pub use output::{plan_output_path, resolve_conflict, write_atomic};
pub use preset::{ConflictPolicy, OutputMode, OutputPolicy, PresetDefaults, ProjectPreset, ScanPolicy, PRESET_FILE};
pub use scan::{is_image_path, scan, walk_images, ScanEntry, ScanOptions, ScanReport};
pub use split::{default_out_dir, sanitize_name, split_file, SplitFile, SplitJob, SplitResult};
