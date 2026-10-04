//! Tiny expression language for control-rate formulas in model files.
//!
//! `"roar_hz * exp2(2 * (rpm - 1))"` is parsed once into a postfix [`Program`] and evaluated
//! once per control block with a fixed-size stack: no allocation, no recursion at run time.
//!
//! Operators: `+ - * / ^ < >` and unary minus. Pure functions: `abs sqrt exp exp2 ln sin cos
//! floor db midi min max pow clamp lerp select smoothstep`. Stateful functions keep their own
//! state per call site: `slew(x, up_s, down_s)`, `lag(x, secs)`, `noise(rate_hz)` (smooth
//! random -1..1), `sh(rate_hz)` (stepped random), `lfo(rate_hz)` (sine), `ramp(rate_hz)` (0..1).

use crate::blocks::{settle_coef, SlowNoise};
use crate::math::{Rng, TAU};

pub(crate) const MAX_STACK: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum F1 {
    Abs,
    Sqrt,
    Exp,
    Exp2,
    Ln,
    Sin,
    Cos,
    Floor,
    Db,
    Midi,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum F2 {
    Min,
    Max,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum F3 {
    Clamp,
    Lerp,
    Select,
    Smoothstep,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StFn {
    Slew,
    Lag,
    Noise,
    Sh,
    Lfo,
    Ramp,
}

impl StFn {
    fn arity(self) -> usize {
        match self {
            StFn::Slew => 3,
            StFn::Lag => 2,
            _ => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    Const(f32),
    Var(u16),
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Neg,
    Lt,
    Gt,
    F1(F1),
    F2(F2),
    F3(F3),
    St(StFn, u16),
}

#[derive(Clone, Debug)]
pub(crate) enum ExprState {
    Follow { y: f32, init: bool },
    Noise(SlowNoise),
    Sh { phase: f32, value: f32, rng: Rng },
    Phase(f32),
}

impl ExprState {
    /// Forget smoothing history so the next evaluation jumps to its target.
    pub(crate) fn snap(&mut self) {
        if let ExprState::Follow { init, .. } = self {
            *init = false;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Program {
    ops: Vec<Op>,
}

impl Program {
    /// A constant program's value, if it is one.
    pub(crate) fn as_const(&self) -> Option<f32> {
        match self.ops.as_slice() {
            [Op::Const(c)] => Some(*c),
            _ => None,
        }
    }

    pub(crate) fn eval(&self, vars: &[f32], states: &mut [ExprState], dt: f32) -> f32 {
        let mut st = [0.0f32; MAX_STACK];
        let mut n = 0usize;
        for op in &self.ops {
            match *op {
                Op::Const(c) => {
                    st[n] = c;
                    n += 1;
                }
                Op::Var(i) => {
                    st[n] = vars[i as usize];
                    n += 1;
                }
                Op::Neg => st[n - 1] = -st[n - 1],
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Pow | Op::Lt | Op::Gt => {
                    let (a, b) = (st[n - 2], st[n - 1]);
                    n -= 1;
                    st[n - 1] = match *op {
                        Op::Add => a + b,
                        Op::Sub => a - b,
                        Op::Mul => a * b,
                        Op::Div => a / b,
                        Op::Pow => a.powf(b),
                        Op::Lt => (a < b) as u8 as f32,
                        _ => (a > b) as u8 as f32,
                    };
                }
                Op::F1(f) => {
                    let a = st[n - 1];
                    st[n - 1] = match f {
                        F1::Abs => a.abs(),
                        F1::Sqrt => a.max(0.0).sqrt(),
                        F1::Exp => a.exp(),
                        F1::Exp2 => a.exp2(),
                        F1::Ln => a.max(1e-30).ln(),
                        F1::Sin => a.sin(),
                        F1::Cos => a.cos(),
                        F1::Floor => a.floor(),
                        F1::Db => crate::math::db_to_gain(a),
                        F1::Midi => crate::math::midi_to_hz(a),
                    };
                }
                Op::F2(f) => {
                    let (a, b) = (st[n - 2], st[n - 1]);
                    n -= 1;
                    st[n - 1] = match f {
                        F2::Min => a.min(b),
                        F2::Max => a.max(b),
                        F2::Pow => a.powf(b),
                    };
                }
                Op::F3(f) => {
                    let (a, b, c) = (st[n - 3], st[n - 2], st[n - 1]);
                    n -= 2;
                    st[n - 1] = match f {
                        F3::Clamp => a.max(b).min(c),
                        F3::Lerp => a + (b - a) * c,
                        F3::Select => {
                            if a > 0.5 {
                                b
                            } else {
                                c
                            }
                        }
                        F3::Smoothstep => {
                            let t = ((c - a) / (b - a)).clamp(0.0, 1.0);
                            t * t * (3.0 - 2.0 * t)
                        }
                    };
                }
                Op::St(f, slot) => {
                    let arity = f.arity();
                    let args = [st[n - arity], st[(n + 1 - arity).min(n - 1)], st[n - 1]];
                    n -= arity - 1;
                    st[n - 1] = step(f, &mut states[slot as usize], args, dt);
                }
            }
        }
        let v = st[0];
        if v.is_finite() {
            v
        } else {
            0.0
        }
    }
}

fn step(f: StFn, state: &mut ExprState, a: [f32; 3], dt: f32) -> f32 {
    match (f, state) {
        (StFn::Slew, ExprState::Follow { y, init }) | (StFn::Lag, ExprState::Follow { y, init }) => {
            let x = a[0];
            if !*init {
                *y = x;
                *init = true;
            } else {
                // slew(x, up, down): a = [x, up, down]; lag(x, secs): a = [x, secs, secs].
                let secs = if f == StFn::Lag || x > *y { a[1] } else { a[2] };
                *y += (x - *y) * settle_coef(secs, dt);
            }
            *y
        }
        (StFn::Noise, ExprState::Noise(n)) => n.advance(a[2], dt),
        (StFn::Sh, ExprState::Sh { phase, value, rng }) => {
            *phase += a[2].max(0.0) * dt;
            if *phase >= 1.0 {
                *phase -= phase.floor();
                *value = rng.next_bipolar();
            }
            *value
        }
        (StFn::Lfo, ExprState::Phase(p)) => {
            *p = (*p + a[2].max(0.0) * dt).fract();
            (*p * TAU).sin()
        }
        (StFn::Ramp, ExprState::Phase(p)) => {
            *p = (*p + a[2].max(0.0) * dt).fract();
            *p
        }
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f32),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Op(char),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Tok::Num(text.parse().map_err(|_| format!("bad number '{text}'"))?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            out.push(match c {
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                ',' => Tok::Comma,
                '+' | '-' | '*' | '/' | '^' | '<' | '>' => Tok::Op(c),
                _ => return Err(format!("unexpected character '{c}'")),
            });
            i += 1;
        }
    }
    Ok(out)
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    ops: Vec<Op>,
    resolve: &'a mut dyn FnMut(&str) -> Option<u16>,
    states: &'a mut Vec<ExprState>,
}

/// Compile `src`. `resolve` maps a variable name to its slot in the value table; stateful
/// calls append their state to `states`.
pub(crate) fn compile(src: &str, resolve: &mut dyn FnMut(&str) -> Option<u16>, states: &mut Vec<ExprState>) -> Result<Program, String> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err("empty expression".into());
    }
    let mut p = Parser { toks, pos: 0, ops: Vec::new(), resolve, states };
    p.compare()?;
    if p.pos != p.toks.len() {
        return Err(format!("unexpected {:?}", p.toks[p.pos]));
    }
    // Verify the stack never overflows.
    let (mut depth, mut max) = (0i32, 0i32);
    for op in &p.ops {
        depth += match op {
            Op::Const(_) | Op::Var(_) => 1,
            Op::Neg | Op::F1(_) => 0,
            Op::F3(_) => -2,
            Op::St(f, _) => 1 - f.arity() as i32,
            _ => -1,
        };
        max = max.max(depth);
    }
    if max as usize > MAX_STACK {
        return Err(format!("expression too deeply nested (needs {max} stack slots, limit {MAX_STACK})"));
    }
    Ok(Program { ops: fold_constant(p.ops) })
}

/// Names of the variables an expression reads (for dependency ordering).
pub(crate) fn variables(src: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut scratch = Vec::new();
    compile(
        src,
        &mut |n| {
            names.push(n.to_string());
            Some(0)
        },
        &mut scratch,
    )?;
    Ok(names)
}

/// Collapse a program with no variables or state into a single constant.
fn fold_constant(ops: Vec<Op>) -> Vec<Op> {
    if ops.iter().any(|o| matches!(o, Op::Var(_) | Op::St(..))) {
        return ops;
    }
    let v = Program { ops: ops.clone() }.eval(&[], &mut [], 0.0);
    vec![Op::Const(v)]
}

impl Parser<'_> {
    fn peek_op(&self) -> Option<char> {
        match self.toks.get(self.pos) {
            Some(Tok::Op(c)) => Some(*c),
            _ => None,
        }
    }

    fn compare(&mut self) -> Result<(), String> {
        self.sum()?;
        if let Some(c @ ('<' | '>')) = self.peek_op() {
            self.pos += 1;
            self.sum()?;
            self.ops.push(if c == '<' { Op::Lt } else { Op::Gt });
        }
        Ok(())
    }

    fn sum(&mut self) -> Result<(), String> {
        self.product()?;
        while let Some(c @ ('+' | '-')) = self.peek_op() {
            self.pos += 1;
            self.product()?;
            self.ops.push(if c == '+' { Op::Add } else { Op::Sub });
        }
        Ok(())
    }

    fn product(&mut self) -> Result<(), String> {
        self.unary()?;
        while let Some(c @ ('*' | '/')) = self.peek_op() {
            self.pos += 1;
            self.unary()?;
            self.ops.push(if c == '*' { Op::Mul } else { Op::Div });
        }
        Ok(())
    }

    fn unary(&mut self) -> Result<(), String> {
        if self.peek_op() == Some('-') {
            self.pos += 1;
            self.unary()?;
            self.ops.push(Op::Neg);
            return Ok(());
        }
        self.atom()?;
        if self.peek_op() == Some('^') {
            self.pos += 1;
            self.unary()?;
            self.ops.push(Op::Pow);
        }
        Ok(())
    }

    fn atom(&mut self) -> Result<(), String> {
        let tok = self.toks.get(self.pos).cloned().ok_or("unexpected end of expression")?;
        self.pos += 1;
        match tok {
            Tok::Num(v) => self.ops.push(Op::Const(v)),
            Tok::LParen => {
                self.compare()?;
                self.expect_rparen()?;
            }
            Tok::Ident(name) => {
                if self.toks.get(self.pos) == Some(&Tok::LParen) {
                    self.pos += 1;
                    return self.call(&name);
                }
                match name.as_str() {
                    "pi" => self.ops.push(Op::Const(core::f32::consts::PI)),
                    "tau" => self.ops.push(Op::Const(TAU)),
                    _ => match (self.resolve)(&name) {
                        Some(i) => self.ops.push(Op::Var(i)),
                        None => return Err(format!("unknown name '{name}'")),
                    },
                }
            }
            other => return Err(format!("unexpected {other:?}")),
        }
        Ok(())
    }

    fn expect_rparen(&mut self) -> Result<(), String> {
        if self.toks.get(self.pos) == Some(&Tok::RParen) {
            self.pos += 1;
            Ok(())
        } else {
            Err("missing ')'".into())
        }
    }

    fn call(&mut self, name: &str) -> Result<(), String> {
        let mut argc = 0;
        if self.toks.get(self.pos) != Some(&Tok::RParen) {
            loop {
                self.compare()?;
                argc += 1;
                if self.toks.get(self.pos) == Some(&Tok::Comma) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect_rparen()?;
        let (arity, op) = match name {
            "abs" => (1, Op::F1(F1::Abs)),
            "sqrt" => (1, Op::F1(F1::Sqrt)),
            "exp" => (1, Op::F1(F1::Exp)),
            "exp2" => (1, Op::F1(F1::Exp2)),
            "ln" => (1, Op::F1(F1::Ln)),
            "sin" => (1, Op::F1(F1::Sin)),
            "cos" => (1, Op::F1(F1::Cos)),
            "floor" => (1, Op::F1(F1::Floor)),
            "db" => (1, Op::F1(F1::Db)),
            "midi" => (1, Op::F1(F1::Midi)),
            "min" => (2, Op::F2(F2::Min)),
            "max" => (2, Op::F2(F2::Max)),
            "pow" => (2, Op::F2(F2::Pow)),
            "clamp" => (3, Op::F3(F3::Clamp)),
            "lerp" => (3, Op::F3(F3::Lerp)),
            "select" => (3, Op::F3(F3::Select)),
            "smoothstep" => (3, Op::F3(F3::Smoothstep)),
            "slew" | "lag" | "noise" | "sh" | "lfo" | "ramp" => {
                let f = match name {
                    "slew" => StFn::Slew,
                    "lag" => StFn::Lag,
                    "noise" => StFn::Noise,
                    "sh" => StFn::Sh,
                    "lfo" => StFn::Lfo,
                    _ => StFn::Ramp,
                };
                let slot = self.states.len();
                if slot >= u16::MAX as usize {
                    return Err("too many stateful function calls".into());
                }
                let seed = 0x7A11_0000u32.wrapping_add(slot as u32 * 7919);
                self.states.push(match f {
                    StFn::Slew | StFn::Lag => ExprState::Follow { y: 0.0, init: false },
                    StFn::Noise => ExprState::Noise(SlowNoise::new(seed)),
                    StFn::Sh => ExprState::Sh { phase: 1.0, value: 0.0, rng: Rng::new(seed) },
                    _ => ExprState::Phase(0.0),
                });
                (f.arity(), Op::St(f, slot as u16))
            }
            _ => return Err(format!("unknown function '{name}'")),
        };
        if argc != arity {
            return Err(format!("{name}() takes {arity} argument(s), got {argc}"));
        }
        self.ops.push(op);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, vars: &[(&str, f32)]) -> f32 {
        let mut states = Vec::new();
        let values: Vec<f32> = vars.iter().map(|v| v.1).collect();
        let p = compile(src, &mut |n| vars.iter().position(|v| v.0 == n).map(|i| i as u16), &mut states).unwrap();
        p.eval(&values, &mut states, 0.001)
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(eval("1 + 2 * 3", &[]), 7.0);
        assert_eq!(eval("(1 + 2) * 3", &[]), 9.0);
        assert_eq!(eval("-2 ^ 2", &[]), -4.0);
        assert_eq!(eval("2 ^ 3 ^ 2", &[]), 512.0);
        assert_eq!(eval("10 / 4 - 0.5", &[]), 2.0);
        assert_eq!(eval("1.5e2 + .5", &[]), 150.5);
        assert_eq!(eval("3 > 2", &[]), 1.0);
    }

    #[test]
    fn functions_and_variables() {
        assert_eq!(eval("lerp(10, 20, t)", &[("t", 0.25)]), 12.5);
        assert_eq!(eval("clamp(x * 4, 0, 1)", &[("x", 0.5)]), 1.0);
        assert_eq!(eval("select(x > 0.5, 7, 9)", &[("x", 0.2)]), 9.0);
        assert!((eval("roar_hz * exp2(2 * (rpm - 1))", &[("roar_hz", 900.0), ("rpm", 0.5)]) - 450.0).abs() < 1e-3);
        assert!((eval("midi(69)", &[]) - 440.0).abs() < 1e-3);
        assert_eq!(eval("1 / 0", &[]), 0.0, "non-finite results are sanitised");
    }

    #[test]
    fn errors_are_reported() {
        let mut s = Vec::new();
        let mut none = |_: &str| None;
        assert!(compile("1 +", &mut none, &mut s).unwrap_err().contains("end of expression"));
        assert!(compile("foo", &mut none, &mut s).unwrap_err().contains("unknown name 'foo'"));
        assert!(compile("lerp(1, 2)", &mut none, &mut s).unwrap_err().contains("takes 3"));
        assert!(compile("wat(1)", &mut none, &mut s).unwrap_err().contains("unknown function"));
        assert!(compile("(1 + 2", &mut none, &mut s).unwrap_err().contains("missing ')'"));
        assert!(compile("2 $ 3", &mut none, &mut s).is_err());
    }

    #[test]
    fn slew_has_inertia_and_direction() {
        let mut states = Vec::new();
        let p = compile("slew(x, 1, 4)", &mut |_| Some(0), &mut states).unwrap();
        assert_eq!(p.eval(&[0.0], &mut states, 0.01), 0.0);
        let mut y = 0.0;
        for _ in 0..100 {
            y = p.eval(&[1.0], &mut states, 0.01);
        }
        assert!(y > 0.93 && y < 1.0, "after 1 s up: {y}");
        for _ in 0..100 {
            y = p.eval(&[0.0], &mut states, 0.01);
        }
        assert!(y > 0.4, "falls slower than it rises: {y}");
    }

    #[test]
    fn variables_lists_dependencies() {
        assert_eq!(variables("a * lerp(b, 2, a)").unwrap(), vec!["a", "b", "a"]);
    }
}
