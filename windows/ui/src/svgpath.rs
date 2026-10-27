// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! A small SVG path-data parser: the glyph and mark drawings are kept as the SVG `d` strings
//! of `docs/design/harness/glyphs.js` and `spec/design/mark/`, converted here to absolute segments.

/// One absolute path segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Cubic Bezier: control 1, control 2, end.
    Cubic(f32, f32, f32, f32, f32, f32),
    Arc {
        rx: f32,
        ry: f32,
        rot: f32,
        large: bool,
        sweep: bool,
        x: f32,
        y: f32,
    },
    Close,
}

struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
}

impl Scanner<'_> {
    fn skip_sep(&mut self) {
        while self.i < self.s.len()
            && (self.s[self.i] == b' '
                || self.s[self.i] == b','
                || self.s[self.i].is_ascii_whitespace())
        {
            self.i += 1;
        }
    }

    fn peek_cmd(&mut self) -> Option<u8> {
        self.skip_sep();
        self.s
            .get(self.i)
            .copied()
            .filter(|c| c.is_ascii_alphabetic())
    }

    fn has_number(&mut self) -> bool {
        self.skip_sep();
        matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit() || *c == b'-' || *c == b'+' || *c == b'.')
    }

    fn number(&mut self) -> Result<f32, String> {
        self.skip_sep();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut seen_dot = false;
        while let Some(&c) = self.s.get(self.i) {
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !seen_dot {
                seen_dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            while matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .map_err(|e| e.to_string())?
            .parse::<f32>()
            .map_err(|_| format!("bad number at {start}"))
    }

    fn flag(&mut self) -> Result<bool, String> {
        self.skip_sep();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(format!("bad arc flag at {}", self.i)),
        }
    }
}

/// Parses SVG path data into absolute segments.
pub fn parse(d: &str) -> Result<Vec<Seg>, String> {
    let mut sc = Scanner {
        s: d.as_bytes(),
        i: 0,
    };
    let mut out = Vec::new();
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    // Last cubic control point, for the smooth `S` command.
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = 0u8;
    loop {
        if let Some(c) = sc.peek_cmd() {
            cmd = c;
            sc.i += 1;
        } else if !sc.has_number() {
            break;
        } else if cmd == 0 {
            return Err("path must start with a command".into());
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        let mut next_ctrl = None;
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Move(x, y));
                (cx, cy, sx, sy) = (x, y, x, y);
                // Further coordinate pairs after a move are implicit lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Line(x, y));
                (cx, cy) = (x, y);
            }
            b'H' => {
                let x = sc.number()? + ox;
                out.push(Seg::Line(x, cy));
                cx = x;
            }
            b'V' => {
                let y = sc.number()? + oy;
                out.push(Seg::Line(cx, y));
                cy = y;
            }
            b'C' => {
                let (x1, y1) = (sc.number()? + ox, sc.number()? + oy);
                let (x2, y2) = (sc.number()? + ox, sc.number()? + oy);
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                next_ctrl = Some((x2, y2));
                (cx, cy) = (x, y);
            }
            b'S' => {
                let (x1, y1) = match last_ctrl {
                    Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                    None => (cx, cy),
                };
                let (x2, y2) = (sc.number()? + ox, sc.number()? + oy);
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                next_ctrl = Some((x2, y2));
                (cx, cy) = (x, y);
            }
            b'A' => {
                let rx = sc.number()?;
                let ry = sc.number()?;
                let rot = sc.number()?;
                let large = sc.flag()?;
                let sweep = sc.flag()?;
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Arc {
                    rx,
                    ry,
                    rot,
                    large,
                    sweep,
                    x,
                    y,
                });
                (cx, cy) = (x, y);
            }
            b'Z' => {
                out.push(Seg::Close);
                (cx, cy) = (sx, sy);
            }
            other => return Err(format!("unsupported command {}", other as char)),
        }
        last_ctrl = next_ctrl;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_shorthand_commands_become_absolute() {
        let s = parse("M12 3c-3.3 0-5.5 2.2-5.5 5.5v7z").unwrap();
        assert_eq!(s[0], Seg::Move(12.0, 3.0));
        assert_eq!(s[1], Seg::Cubic(8.7, 3.0, 6.5, 5.2, 6.5, 8.5));
        assert_eq!(s[2], Seg::Line(6.5, 15.5));
        assert_eq!(s[3], Seg::Close);
    }

    #[test]
    fn smooth_curve_reflects_the_previous_control_point() {
        let s = parse("M0 0C1 1 2 1 3 0s2 -1 3 0").unwrap();
        assert_eq!(s[2], Seg::Cubic(4.0, -1.0, 5.0, -1.0, 6.0, 0.0));
    }

    #[test]
    fn numbers_may_run_together() {
        let s = parse("M0 0c0 1.4.3 2.7 1 3.8").unwrap();
        assert_eq!(s[1], Seg::Cubic(0.0, 1.4, 0.3, 2.7, 1.0, 3.8));
        let s = parse("M1 1l-1-1").unwrap();
        assert_eq!(s[1], Seg::Line(0.0, 0.0));
    }

    #[test]
    fn arcs_carry_flags_and_relative_endpoints() {
        let s = parse("M4.5 14v-2a7.5 7.5 0 0 1 15 0v2").unwrap();
        assert_eq!(
            s[2],
            Seg::Arc {
                rx: 7.5,
                ry: 7.5,
                rot: 0.0,
                large: false,
                sweep: true,
                x: 19.5,
                y: 12.0
            }
        );
        assert_eq!(s[3], Seg::Line(19.5, 14.0));
    }

    #[test]
    fn implicit_lineto_after_move_and_h_v() {
        let s = parse("M1 1 2 2H5V0").unwrap();
        assert_eq!(
            s,
            vec![
                Seg::Move(1.0, 1.0),
                Seg::Line(2.0, 2.0),
                Seg::Line(5.0, 2.0),
                Seg::Line(5.0, 0.0)
            ]
        );
    }

    #[test]
    fn dots_are_tiny_horizontal_lines() {
        let s = parse("M6 9h.01M9 9h.01").unwrap();
        assert_eq!(s.len(), 4);
        assert_eq!(s[1], Seg::Line(6.01, 9.0));
        assert_eq!(s[2], Seg::Move(9.0, 9.0));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(parse("12 3").is_err());
        assert!(parse("M1 1X").is_err());
        assert!(parse("M1").is_err());
    }
}
