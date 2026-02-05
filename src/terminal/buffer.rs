pub struct TerminalBuffer {
    lines: Vec<String>,
    max_scrollback: usize,
}

impl TerminalBuffer {
    pub fn new(max_scrollback: usize) -> Self {
        Self {
            lines: Vec::new(),
            max_scrollback,
        }
    }

    pub fn push_line(&mut self, line: String) {
        if self.lines.len() >= self.max_scrollback {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }

    pub fn push_output(&mut self, chunk: &str) {
        let normalized = strip_ansi(chunk).replace("\r\n", "\n").replace('\r', "\n");
        let mut iter = normalized.split('\n').peekable();

        if let Some(first) = iter.next() {
            if self.lines.is_empty() {
                self.lines.push(first.to_string());
            } else {
                if let Some(last) = self.lines.last_mut() {
                    last.push_str(first);
                }
            }
        }

        for part in iter {
            self.push_line(part.to_string());
        }
    }

    pub fn get_lines(&self) -> &[String] {
        &self.lines
    }
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if let Some(next) = chars.peek() {
                match *next {
                    '[' => {
                        // CSI: consume until a letter in @-~
                        chars.next();
                        while let Some(c) = chars.next() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    ']' => {
                        // OSC: consume until BEL or ST
                        chars.next();
                        let mut prev = '\0';
                        while let Some(c) = chars.next() {
                            if c == '\x07' || (prev == '\x1b' && c == '\\') {
                                break;
                            }
                            prev = c;
                        }
                    }
                    _ => {
                        // Other ESC sequence: consume next char
                        continue;
                    }
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}
