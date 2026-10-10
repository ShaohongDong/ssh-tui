use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

pub fn is_prefix(key: KeyEvent) -> bool {
    // Legacy terminals encode Ctrl+] and Ctrl+5 as the same byte (0x1d).
    matches!(key.code, KeyCode::Char(']') | KeyCode::Char('5'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub fn encode(key: KeyEvent, application_cursor: bool, application_keypad: bool) -> Vec<u8> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let mut result = match key.code {
        KeyCode::Char(c) if ctrl => {
            let byte = match c.to_ascii_lowercase() {
                '@' | ' ' | '2' => 0,
                'a'..='z' => c.to_ascii_lowercase() as u8 - b'a' + 1,
                '[' | '3' => 27,
                '\\' | '4' => 28,
                ']' | '5' => 29,
                '^' | '6' => 30,
                '_' | '/' | '7' => 31,
                '?' | '8' => 127,
                _ => return c.to_string().into_bytes(),
            };
            vec![byte]
        }
        KeyCode::Char(c) if application_keypad && key.state.contains(KeyEventState::KEYPAD) => {
            let code = match c {
                '0'..='9' => b'p' + c as u8 - b'0',
                '.' => b'n',
                '/' => b'o',
                '*' => b'j',
                '-' => b'm',
                '+' => b'k',
                ',' => b'l',
                '=' => b'X',
                _ => return c.to_string().into_bytes(),
            };
            vec![27, b'O', code]
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter if application_keypad && key.state.contains(KeyEventState::KEYPAD) => {
            b"\x1bOM".to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![if ctrl { 8 } else { 127 }],
        KeyCode::Esc => vec![27],
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Right
        | KeyCode::Left
        | KeyCode::Home
        | KeyCode::End => {
            let final_char = match key.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{final_char}").into_bytes()
            } else {
                format!(
                    "\x1b{}{final_char}",
                    if application_cursor { 'O' } else { '[' }
                )
                .into_bytes()
            };
        }
        KeyCode::Insert | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown => {
            let n = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                _ => 6,
            };
            return tilde(n, modifier);
        }
        KeyCode::F(n @ 1..=4) => {
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{}", (b'P' + n - 1) as char).into_bytes()
            } else {
                vec![27, b'O', b'P' + n - 1]
            };
        }
        KeyCode::F(n @ 5..=12) => {
            return tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize], modifier);
        }
        _ => Vec::new(),
    };
    if alt && !result.is_empty() {
        result.insert(0, 27);
    }
    result
}

fn tilde(n: u8, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1b[{n}~").into_bytes()
    } else {
        format!("\x1b[{n};{modifier}~").into_bytes()
    }
}

pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\n").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_and_cursor_modes() {
        assert!(is_prefix(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::CONTROL
        )));
        for (c, b) in [
            ('c', 3),
            ('d', 4),
            ('z', 26),
            ('l', 12),
            (']', 29),
            (' ', 0),
        ] {
            assert_eq!(
                encode(
                    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
                    false,
                    false
                ),
                [b]
            );
        }
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(encode(up, false, false), b"\x1b[A");
        assert_eq!(encode(up, true, false), b"\x1bOA");
        assert_eq!(
            encode(
                KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
                false,
                false
            ),
            b"\x1b[1;5C"
        );
        assert_eq!(paste("a\nb", true), b"\x1b[200~a\nb\x1b[201~");
    }
}
