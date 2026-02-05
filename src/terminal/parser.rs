use vte::{Parser, Perform, Params};

pub struct TerminalParser {
    parser: Parser,
}

impl TerminalParser {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
        }
    }

    pub fn process(&mut self, data: &[u8], performer: &mut impl Perform) {
        for byte in data {
            self.parser.advance(performer, *byte);
        }
    }
}

pub struct SemanticPerformer {
    // This will hold the state to detect OSC 133
}

impl SemanticPerformer {
    pub fn new() -> Self {
        Self {}
    }
}

impl Perform for SemanticPerformer {
    fn print(&mut self, _c: char) {}
    fn execute(&mut self, _byte: u8) {}
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.len() > 0 && params[0].starts_with(b"133;") {
            let cmd = &params[0][4..];
            match cmd {
                b"A" => println!("Prompt started"),
                b"B" => println!("Command started"),
                b"C" => println!("Command output started"),
                b"D" => println!("Command finished"),
                _ => {}
            }
        }
    }
    fn csi_dispatch(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
}
