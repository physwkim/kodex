use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

static SENSITIVE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)(^|[\\/])\.(env|envrc)(\.|$)").unwrap(),
        Regex::new(r"(?i)\.(pem|key|p12|pfx|cert|crt|der|p8)$").unwrap(),
        Regex::new(r"(?i)(credential|secret|passwd|password|token|private_key)").unwrap(),
        Regex::new(r"(id_rsa|id_dsa|id_ecdsa|id_ed25519)(\.pub)?$").unwrap(),
        Regex::new(r"(?i)(\.netrc|\.pgpass|\.htpasswd)$").unwrap(),
        Regex::new(r"(?i)(aws_credentials|gcloud_credentials|service.account)").unwrap(),
    ]
});

/// Returns true if the file path matches any sensitive-file pattern.
pub fn is_sensitive(path: &Path) -> bool {
    let s = path.to_string_lossy();
    SENSITIVE_PATTERNS.iter().any(|pat| pat.is_match(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_env_file() {
        assert!(is_sensitive(&PathBuf::from(".env")));
        assert!(is_sensitive(&PathBuf::from(".env.local")));
        assert!(is_sensitive(&PathBuf::from("config/.envrc")));
    }

    #[test]
    fn test_key_files() {
        assert!(is_sensitive(&PathBuf::from("server.pem")));
        assert!(is_sensitive(&PathBuf::from("id_rsa")));
        assert!(is_sensitive(&PathBuf::from("id_rsa.pub")));
    }

    #[test]
    fn test_credential_files() {
        assert!(is_sensitive(&PathBuf::from("aws_credentials")));
        assert!(is_sensitive(&PathBuf::from("credentials.json")));
    }

    #[test]
    fn test_normal_files() {
        assert!(!is_sensitive(&PathBuf::from("main.py")));
        assert!(!is_sensitive(&PathBuf::from("README.md")));
    }
}
