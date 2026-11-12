//! ZapretGUI.ini — the same key=value format the C# version used, so settings carry over.

use std::fs;
use std::path::PathBuf;

pub struct Cfg {
    path: PathBuf,
    items: Vec<(String, String)>,
}

impl Cfg {
    pub fn load(path: PathBuf) -> Self {
        let mut items = Vec::new();
        if let Ok(bytes) = fs::read(&path) {
            let text = String::from_utf8_lossy(&bytes);
            for raw in text.lines() {
                let line = raw.trim().trim_start_matches('\u{feff}');
                if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                    continue;
                }
                if let Some(i) = line.find('=') {
                    if i == 0 {
                        continue;
                    }
                    let (k, v) = (line[..i].trim().to_string(), line[i + 1..].trim().to_string());
                    match items.iter_mut().find(|(ek, _): &&mut (String, String)| ek.eq_ignore_ascii_case(&k)) {
                        Some(e) => e.1 = v,
                        None => items.push((k, v)),
                    }
                }
            }
        }
        Cfg { path, items }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.items.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn get_or(&self, key: &str, def: &str) -> String {
        self.get(key).unwrap_or(def).to_string()
    }

    pub fn get_bool(&self, key: &str, def: bool) -> bool {
        match self.get(key) {
            Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
            None => def,
        }
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.items.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(e) => e.1 = value,
            None => self.items.push((key.to_string(), value)),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.items.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.set(key, if value { "1" } else { "0" });
    }

    pub fn save(&self) {
        let mut s = String::from("# ZapretGUI settings\r\n");
        for (k, v) in &self.items {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push_str("\r\n");
        }
        let _ = fs::write(&self.path, s);
    }
}
