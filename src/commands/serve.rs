use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Start MCP stdio server. Connects to actor (starts it if needed).
/// The graph_path argument is unused — actor always uses ~/.kodex/kodex.h5.
pub fn serve(_graph_path: &Path) {
    // Ensure actor is running
    if let Err(e) = kodex::actor::ensure_running() {
        eprintln!("Failed to start actor: {e}");
        return;
    }

    // Connect to actor socket
    let sock_path = kodex::actor::socket_path();
    let stream = match connect_with_retry(&sock_path, 10) {
        Some(s) => s,
        None => {
            eprintln!("Failed to connect to actor at {}", sock_path.display());
            return;
        }
    };

    let writer_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to clone stream: {e}");
            return;
        }
    };

    // Get CWD for project context
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    // Proxy: stdin → actor, actor → stdout
    let mut actor_writer = writer_stream;
    let actor_reader = BufReader::new(stream);

    // Spawn reader thread: actor → stdout
    let reader_handle = std::thread::spawn(move || {
        for line in actor_reader.lines() {
            match line {
                Ok(l) => {
                    println!("{l}");
                    let _ = std::io::stdout().flush();
                }
                Err(_) => break,
            }
        }
    });

    // Main thread: stdin → actor (inject project_dir)
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Inject project_dir into params
                let enriched = inject_project_dir(trimmed, &cwd);
                if writeln!(actor_writer, "{enriched}").is_err() {
                    break;
                }
                let _ = actor_writer.flush();
            }
            Err(_) => break,
        }
    }

    let _ = reader_handle.join();
}

/// Connect to Unix socket with retries.
fn connect_with_retry(path: &Path, max_retries: u32) -> Option<std::os::unix::net::UnixStream> {
    for _ in 0..max_retries {
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(path) {
            return Some(stream);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

/// Inject project_dir into JSON-RPC params so actor knows which h5 to use.
fn inject_project_dir(input: &str, cwd: &str) -> String {
    let mut req: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(_) => return input.to_string(),
    };

    if let Some(obj) = req.as_object_mut() {
        let params = obj.entry("params").or_insert_with(|| serde_json::json!({}));
        if let Some(p) = params.as_object_mut() {
            p.entry("project_dir")
                .or_insert_with(|| serde_json::json!(cwd));
        }
    }

    serde_json::to_string(&req).unwrap_or_else(|_| input.to_string())
}
