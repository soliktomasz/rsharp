//! Xterm's keys and paste, independent of the UI's keystroke types.

pub fn key(key: &str, ctrl: bool, alt: bool, shift: bool, application: bool) -> Option<String> {
    let modifier = 1 + usize::from(shift) + 2 * usize::from(alt) + 4 * usize::from(ctrl);
    let cursor = match key {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(end) = cursor {
        return Some(if modifier > 1 {
            format!("\x1b[1;{modifier}{end}")
        } else if application {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[{end}")
        });
    }
    let tilde = match key {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    };
    if let Some(code) = tilde {
        return Some(if modifier > 1 { format!("\x1b[{code};{modifier}~") } else { format!("\x1b[{code}~") });
    }
    let function = match key {
        "f1" => Some('P'),
        "f2" => Some('Q'),
        "f3" => Some('R'),
        "f4" => Some('S'),
        _ => None,
    };
    if let Some(code) = function {
        return Some(if modifier > 1 { format!("\x1b[1;{modifier}{code}") } else { format!("\x1bO{code}") });
    }
    let text = match key {
        "enter" => "\r".into(),
        "escape" => "\x1b".into(),
        "backspace" => if ctrl { "\x08" } else { "\x7f" }.into(),
        "tab" => if shift { "\x1b[Z" } else { "\t" }.into(),
        "space" if ctrl => "\0".into(),
        _ if ctrl && key.len() == 1 => {
            let c = key.as_bytes()[0].to_ascii_uppercase();
            if (b'@'..=b'_').contains(&c) {
                char::from(c & 0x1f).to_string()
            } else if c == b'?' {
                "\x7f".into()
            } else {
                return None;
            }
        }
        _ if alt && key.chars().count() == 1 => key.to_string(),
        _ => return None,
    };
    Some(if alt { format!("\x1b{text}") } else { text })
}

pub fn paste(text: &str, bracketed: bool) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                clean.push('\r');
            }
            '\n' => clean.push('\r'),
            '\t' => clean.push(c),
            c if !c.is_control() => clean.push(c),
            _ => {}
        }
    }
    if bracketed && !clean.is_empty() { format!("\x1b[200~{clean}\x1b[201~") } else { clean }
}

pub fn mouse(
    button: u8,
    col: usize,
    row: usize,
    release: bool,
    sgr: bool,
    shift: bool,
    alt: bool,
    ctrl: bool,
) -> Option<String> {
    let button = button + u8::from(shift) * 4 + u8::from(alt) * 8 + u8::from(ctrl) * 16;
    if sgr {
        Some(format!("\x1b[<{button};{};{}{}", col + 1, row + 1, if release { 'm' } else { 'M' }))
    } else if col < 223 && row < 223 {
        Some(
            [0x1b, b'[', b'M', if release { 35 } else { 32 + button }, 33 + col as u8, 33 + row as u8]
                .into_iter()
                .map(char::from)
                .collect(),
        )
    } else {
        None
    }
}
