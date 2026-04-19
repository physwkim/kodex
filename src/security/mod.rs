mod label;
mod path;
mod url;

pub use self::label::sanitize_label;
pub use self::path::validate_graph_path;
pub use self::url::validate_url;

/// Maximum binary download size (50 MB).
pub const MAX_FETCH_BYTES: usize = 52_428_800;

/// Maximum text download size (10 MB).
pub const MAX_TEXT_BYTES: usize = 10_485_760;
