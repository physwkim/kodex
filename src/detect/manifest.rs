use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const MANIFEST_PATH: &str = "graphify-out/manifest.json";

/// Load file manifest (path → mtime) from `graphify-out/manifest.json`.
pub fn load_manifest(root: &Path) -> Option<HashMap<String, f64>> {
    let manifest_file = root.join(MANIFEST_PATH);
    let content = std::fs::read_to_string(manifest_file).ok()?;
    let val: Value = serde_json::from_str(&content).ok()?;
    let obj = val.as_object()?;

    let mut map = HashMap::new();
    for (k, v) in obj {
        if let Some(mtime) = v.as_f64() {
            map.insert(k.clone(), mtime);
        }
    }
    Some(map)
}

/// Save file manifest (path → mtime) to `graphify-out/manifest.json`.
pub fn save_manifest(files: &HashMap<String, f64>, root: &Path) -> std::io::Result<()> {
    let manifest_file = root.join(MANIFEST_PATH);
    if let Some(parent) = manifest_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(files)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(manifest_file, json)
}
