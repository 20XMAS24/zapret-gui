use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Info,
    Ok,
    Warn,
    Error,
    Dim,
    Head,
}

#[derive(Clone)]
pub struct Line {
    pub time: String,
    pub text: String,
    pub kind: Kind,
}

#[derive(Default)]
pub struct LogBuf {
    pub lines: VecDeque<Line>,
}

impl LogBuf {
    const MAX: usize = 1500;

    pub fn push(&mut self, text: &str, kind: Kind) {
        for part in text.lines() {
            let part = part.trim_end();
            if part.is_empty() && text.lines().count() > 1 {
                continue;
            }
            self.lines.push_back(Line { time: crate::sys::now_hms(), text: part.to_string(), kind });
        }
        while self.lines.len() > Self::MAX {
            self.lines.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn as_text(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(&l.time);
            s.push_str("  ");
            s.push_str(&l.text);
            s.push_str("\r\n");
        }
        s
    }
}
