use std::path::PathBuf;

use super::message::ConversationMessage;

fn conversations_dir() -> PathBuf {
    let base = if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                PathBuf::from(home).join(".config")
            })
    } else if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(".")
    };
    let dir = base.join("epubthing").join("conversations");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn conversation_filename(book_path: &str) -> String {
    let name = std::path::Path::new(book_path)
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into());
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.json", safe)
}

pub fn load_conversation(book_path: &str) -> Vec<ConversationMessage> {
    let path = conversations_dir().join(conversation_filename(book_path));
    if path.exists() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(msgs) = serde_json::from_str(&json) {
                return msgs;
            }
        }
    }
    Vec::new()
}

pub fn save_conversation(book_path: &str, messages: &[ConversationMessage]) {
    let path = conversations_dir().join(conversation_filename(book_path));
    if let Ok(json) = serde_json::to_string_pretty(messages) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn export_conversation(
    messages: &[ConversationMessage],
    export_path: &str,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(messages).map_err(|e| format!("{}", e))?;
    std::fs::write(export_path, json).map_err(|e| format!("{}", e))
}

pub fn import_conversation(
    book_path: &str,
    import_path: &str,
) -> Result<Vec<ConversationMessage>, String> {
    let json = std::fs::read_to_string(import_path).map_err(|e| format!("{}", e))?;
    let messages: Vec<ConversationMessage> =
        serde_json::from_str(&json).map_err(|e| format!("{}", e))?;
    save_conversation(book_path, &messages);
    Ok(messages)
}
