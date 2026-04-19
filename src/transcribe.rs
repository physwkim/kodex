use std::path::{Path, PathBuf};

/// Check if a string looks like a URL.
pub fn is_url(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://") || path.starts_with("www.")
}

/// Download audio from a URL using yt-dlp (shell out).
pub fn download_audio(url: &str, output_dir: &Path) -> crate::error::Result<PathBuf> {
    std::fs::create_dir_all(output_dir)?;

    let output = std::process::Command::new("yt-dlp")
        .args([
            "-x",
            "--audio-format", "wav",
            "--audio-quality", "0",
            "-o", &format!("{}/%(title)s.%(ext)s", output_dir.display()),
            url,
        ])
        .output()
        .map_err(|e| {
            crate::error::KodexError::Other(format!(
                "yt-dlp not found: {e}. Install with: pip install yt-dlp"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::KodexError::Other(format!(
            "yt-dlp failed: {stderr}"
        )));
    }

    // Find the most recently created audio file
    let mut entries: Vec<_> = std::fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| {
                    let s = ext.to_string_lossy().to_lowercase();
                    ["wav", "mp3", "m4a", "opus", "ogg", "webm"].contains(&s.as_str())
                })
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    entries
        .last()
        .map(|e| e.path())
        .ok_or_else(|| {
            crate::error::KodexError::Other("No audio file found after download".to_string())
        })
}

/// Build a Whisper prompt from god node labels for domain-aware transcription.
pub fn build_whisper_prompt(god_node_labels: &[String]) -> String {
    // Allow override via env var
    if let Ok(custom) = std::env::var("KODEX_WHISPER_PROMPT") {
        if !custom.is_empty() {
            return custom;
        }
    }

    if god_node_labels.is_empty() {
        return "Use proper punctuation and paragraph breaks.".to_string();
    }
    let terms: String = god_node_labels
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    format!("This audio discusses: {terms}. Use proper punctuation and paragraph breaks.")
}

/// Transcribe an audio/video file to text using whisper.cpp.
///
/// Returns the path to the generated transcript file.
/// Requires the `video` feature.
#[cfg(feature = "video")]
pub fn transcribe(
    audio_path: &Path,
    output_dir: Option<&Path>,
    initial_prompt: Option<&str>,
    force: bool,
) -> crate::error::Result<PathBuf> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let out_dir = output_dir.unwrap_or_else(|| Path::new("kodex-out/transcripts"));
    std::fs::create_dir_all(out_dir)?;

    let stem = audio_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let transcript_path = out_dir.join(format!("{stem}.txt"));

    // Check cache
    if transcript_path.exists() && !force {
        return Ok(transcript_path);
    }

    // Determine model path
    let model_name = std::env::var("KODEX_WHISPER_MODEL").unwrap_or_else(|_| "base".to_string());
    let model_path = resolve_model_path(&model_name)?;

    // Load model
    let ctx = WhisperContext::new_with_params(&model_path, WhisperContextParameters::default())
        .map_err(|e| crate::error::KodexError::Other(format!("Failed to load Whisper model: {e}")))?;

    // Read and convert audio to 16kHz mono f32 PCM
    let samples = load_audio_samples(audio_path)?;

    // Configure transcription
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("auto"));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    if let Some(prompt) = initial_prompt {
        params.set_initial_prompt(prompt);
    }

    // Run inference
    let mut state = ctx.create_state()
        .map_err(|e| crate::error::KodexError::Other(format!("Failed to create state: {e}")))?;

    state.full(params, &samples)
        .map_err(|e| crate::error::KodexError::Other(format!("Transcription failed: {e}")))?;

    // Collect segments
    let num_segments = state.full_n_segments();

    let mut transcript = String::new();
    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            if let Ok(text) = segment.to_str_lossy() {
                transcript.push_str(text.trim());
                transcript.push('\n');
            }
        }
    }

    std::fs::write(&transcript_path, transcript.trim())?;
    Ok(transcript_path)
}

/// Transcribe a list of audio/video files or URLs.
#[cfg(feature = "video")]
pub fn transcribe_all(
    files: &[String],
    output_dir: Option<&Path>,
    initial_prompt: Option<&str>,
) -> Vec<Result<PathBuf, String>> {
    let out_dir = output_dir.unwrap_or_else(|| Path::new("kodex-out/transcripts"));

    files
        .iter()
        .map(|file| {
            let path = if is_url(file) {
                download_audio(file, out_dir).map_err(|e| e.to_string())?
            } else {
                PathBuf::from(file)
            };
            transcribe(&path, Some(out_dir), initial_prompt, false).map_err(|e| e.to_string())
        })
        .collect()
}

/// Load audio file as 16kHz mono f32 samples.
/// Supports WAV directly via hound; other formats need ffmpeg conversion.
#[cfg(feature = "video")]
fn load_audio_samples(path: &Path) -> crate::error::Result<Vec<f32>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let wav_path = if ext == "wav" {
        path.to_path_buf()
    } else {
        // Convert to WAV using ffmpeg
        let tmp = path.with_extension("_16k.wav");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y", "-i",
                path.to_str().unwrap_or(""),
                "-ar", "16000",
                "-ac", "1",
                "-f", "wav",
                tmp.to_str().unwrap_or(""),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| {
                crate::error::KodexError::Other(format!(
                    "ffmpeg not found: {e}. Install ffmpeg to convert non-WAV audio."
                ))
            })?;

        if !status.success() {
            return Err(crate::error::KodexError::Other(
                "ffmpeg conversion failed".to_string(),
            ));
        }
        tmp
    };

    // Read WAV with hound
    let reader = hound::WavReader::open(&wav_path)
        .map_err(|e| crate::error::KodexError::Other(format!("Failed to read WAV: {e}")))?;

    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .into_samples::<i16>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i16::MAX as f32)
            .collect(),
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    // Resample to 16kHz if needed
    if spec.sample_rate != 16000 {
        let ratio = 16000.0 / spec.sample_rate as f64;
        let new_len = (samples.len() as f64 * ratio) as usize;
        let mut resampled = Vec::with_capacity(new_len);
        for i in 0..new_len {
            let src_idx = (i as f64 / ratio) as usize;
            if src_idx < samples.len() {
                resampled.push(samples[src_idx]);
            }
        }
        Ok(resampled)
    } else {
        Ok(samples)
    }
}

/// Resolve Whisper model path — look in common locations or download hint.
#[cfg(feature = "video")]
fn resolve_model_path(model_name: &str) -> crate::error::Result<String> {
    // Check env var for explicit path
    if let Ok(path) = std::env::var("KODEX_WHISPER_MODEL_PATH") {
        if Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // Common locations for ggml models
    let filename = format!("ggml-{model_name}.bin");
    let candidates = [
        // Current directory
        PathBuf::from(&filename),
        // ~/.cache/whisper/
        dirs::home_dir()
            .unwrap_or_default()
            .join(".cache/whisper")
            .join(&filename),
        // ~/.local/share/whisper/
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/share/whisper")
            .join(&filename),
        // XDG data dir
        dirs::data_dir()
            .unwrap_or_default()
            .join("whisper")
            .join(&filename),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().to_string());
        }
    }

    Err(crate::error::KodexError::Other(format!(
        "Whisper model '{filename}' not found.\n\
         Download from: https://huggingface.co/ggerganov/whisper.cpp/tree/main\n\
         Place in ~/.cache/whisper/ or set KODEX_WHISPER_MODEL_PATH env var."
    )))
}

// Stub when video feature is not enabled
#[cfg(not(feature = "video"))]
pub fn transcribe(
    _audio_path: &Path,
    _output_dir: Option<&Path>,
    _initial_prompt: Option<&str>,
    _force: bool,
) -> crate::error::Result<PathBuf> {
    Err(crate::error::KodexError::Other(
        "Transcription requires --features video. Rebuild with: cargo build --features video"
            .to_string(),
    ))
}
