//! Safe, bounded expressions for portable user-defined radar products.

use crate::level2::Moment;

const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 16;
const MAX_COLUMN_GATES: usize = 4096;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProductDefinition {
    pub version: u16,
    pub name: String,
    pub description: String,
    pub units: String,
    pub inputs: Vec<Moment>,
    pub expression: String,
    pub palette: String,
    pub min: f32,
    pub max: f32,
    pub missing: f32,
    #[serde(default)]
    pub environment: Vec<EnvironmentInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentInput {
    FreezingLevel,
    Minus10CHeight,
    Minus20CHeight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Number,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Var {
    Moment(Moment),
    Altitude,
    Range,
    Azimuth,
    Elevation,
    Freezing,
    Minus10,
    Minus20,
}

#[derive(Debug, Clone)]
enum Expr {
    Number(f32),
    Var(Var),
    Unary(bool, Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call(Fn, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy)]
enum Fn {
    Min,
    Max,
    Mean,
    Clamp,
    If,
    Threshold,
    MaxVertical,
    MinVertical,
    MeanVertical,
    MaxLayer,
    MinLayer,
    MeanLayer,
    FirstHeight,
    LastHeight,
    CountGates,
}

#[derive(Debug, Clone, Copy)]
enum Value {
    Number(f32),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GateValues {
    pub moments: [Option<f32>; 7],
    pub altitude_km: f32,
    pub range_km: f32,
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub freezing_km: Option<f32>,
    pub minus10_km: Option<f32>,
    pub minus20_km: Option<f32>,
}

pub struct Product {
    definition: ProductDefinition,
    root: Expr,
}

impl Product {
    pub fn compile(definition: ProductDefinition) -> anyhow::Result<Self> {
        anyhow::ensure!(!definition.name.trim().is_empty(), "product name is empty");
        anyhow::ensure!(definition.version == 1, "unsupported product definition version");
        anyhow::ensure!(definition.min < definition.max, "product range is empty");
        anyhow::ensure!(definition.expression.len() <= 2048, "expression is too long");
        let mut parser = Parser::new(&definition.expression)?;
        let root = parser.expression(0, 0)?;
        anyhow::ensure!(parser.peek().is_none(), "unexpected trailing input");
        anyhow::ensure!(root.ty()? == Ty::Number, "product must produce a number");
        let mut used = Vec::new();
        root.moments(&mut used);
        anyhow::ensure!(
            used.into_iter().all(|moment| definition.inputs.contains(&moment)),
            "expression uses an undeclared radar moment"
        );
        let mut environment = Vec::new();
        root.environment(&mut environment);
        anyhow::ensure!(
            environment
                .into_iter()
                .all(|input| definition.environment.contains(&input)),
            "expression uses an undeclared environmental input"
        );
        Ok(Self { definition, root })
    }

    pub fn definition(&self) -> &ProductDefinition {
        &self.definition
    }

    pub fn sample(&self, gate: &GateValues) -> f32 {
        match self.root.eval(gate) {
            Some(Value::Number(value)) if value.is_finite() => value,
            _ => self.definition.missing,
        }
    }

    pub fn column(&self, gates: &[GateValues]) -> f32 {
        if gates.is_empty() || gates.len() > MAX_COLUMN_GATES {
            return self.definition.missing;
        }
        match &self.root {
            Expr::Call(function, args) => function
                .eval_column(args, gates)
                .filter(|value| value.is_finite())
                .unwrap_or(self.definition.missing),
            _ if gates.len() == 1 => self.sample(&gates[0]),
            _ => self.definition.missing,
        }
    }
}

impl Expr {
    fn ty(&self) -> anyhow::Result<Ty> {
        match self {
            Self::Number(_) | Self::Var(_) => Ok(Ty::Number),
            Self::Unary(_, value) => {
                anyhow::ensure!(value.ty()? == Ty::Number, "unary operator needs a number");
                Ok(Ty::Number)
            }
            Self::Binary(op, left, right) => {
                anyhow::ensure!(
                    left.ty()? == Ty::Number && right.ty()? == Ty::Number,
                    "binary operator needs numbers"
                );
                Ok(if matches!(op, Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Eq | Op::Ne) {
                    Ty::Bool
                } else {
                    Ty::Number
                })
            }
            Self::Call(function, args) => function.ty(args),
        }
    }

    fn moments(&self, out: &mut Vec<Moment>) {
        match self {
            Self::Var(Var::Moment(moment)) => out.push(*moment),
            Self::Unary(_, value) => value.moments(out),
            Self::Binary(_, left, right) => {
                left.moments(out);
                right.moments(out);
            }
            Self::Call(_, args) => args.iter().for_each(|arg| arg.moments(out)),
            _ => {}
        }
    }

    fn environment(&self, out: &mut Vec<EnvironmentInput>) {
        match self {
            Self::Var(Var::Freezing) => out.push(EnvironmentInput::FreezingLevel),
            Self::Var(Var::Minus10) => out.push(EnvironmentInput::Minus10CHeight),
            Self::Var(Var::Minus20) => out.push(EnvironmentInput::Minus20CHeight),
            Self::Unary(_, value) => value.environment(out),
            Self::Binary(_, left, right) => {
                left.environment(out);
                right.environment(out);
            }
            Self::Call(_, args) => args.iter().for_each(|arg| arg.environment(out)),
            _ => {}
        }
    }

    fn eval(&self, gate: &GateValues) -> Option<Value> {
        match self {
            Self::Number(value) => Some(Value::Number(*value)),
            Self::Var(var) => Some(Value::Number(var.read(gate)?)),
            Self::Unary(negative, value) => match value.eval(gate)? {
                Value::Number(value) => Some(Value::Number(if *negative { -value } else { value })),
                Value::Bool(_) => None,
            },
            Self::Binary(op, left, right) => {
                let Value::Number(left) = left.eval(gate)? else { return None };
                let Value::Number(right) = right.eval(gate)? else { return None };
                op.eval(left, right)
            }
            Self::Call(function, args) => function.eval(args, gate),
        }
    }
}

impl Var {
    fn read(self, gate: &GateValues) -> Option<f32> {
        match self {
            Self::Moment(moment) => gate.moments[moment.index()],
            Self::Altitude => Some(gate.altitude_km),
            Self::Range => Some(gate.range_km),
            Self::Azimuth => Some(gate.azimuth_deg),
            Self::Elevation => Some(gate.elevation_deg),
            Self::Freezing => gate.freezing_km,
            Self::Minus10 => gate.minus10_km,
            Self::Minus20 => gate.minus20_km,
        }
    }
}

impl Op {
    fn eval(self, a: f32, b: f32) -> Option<Value> {
        Some(match self {
            Self::Add => Value::Number(a + b),
            Self::Sub => Value::Number(a - b),
            Self::Mul => Value::Number(a * b),
            Self::Div if b != 0.0 => Value::Number(a / b),
            Self::Div => return None,
            Self::Lt => Value::Bool(a < b),
            Self::Le => Value::Bool(a <= b),
            Self::Gt => Value::Bool(a > b),
            Self::Ge => Value::Bool(a >= b),
            Self::Eq => Value::Bool(a == b),
            Self::Ne => Value::Bool(a != b),
        })
    }
}

impl Fn {
    fn ty(self, args: &[Expr]) -> anyhow::Result<Ty> {
        let expected = match self {
            Self::Clamp | Self::If => 3,
            Self::Threshold => 2,
            Self::Min | Self::Max | Self::Mean => 2,
            Self::MaxVertical | Self::MinVertical | Self::MeanVertical => {
                anyhow::ensure!((1..=2).contains(&args.len()), "vertical reducer expects value and optional condition");
                anyhow::ensure!(args[0].ty()? == Ty::Number, "vertical value must be numeric");
                if args.len() == 2 { anyhow::ensure!(args[1].ty()? == Ty::Bool, "vertical condition must be boolean"); }
                return Ok(Ty::Number);
            }
            Self::MaxLayer | Self::MinLayer | Self::MeanLayer => {
                anyhow::ensure!((3..=4).contains(&args.len()), "layer reducer expects value, lower, upper, and optional condition");
                anyhow::ensure!(args[..3].iter().all(|arg| arg.ty().ok() == Some(Ty::Number)), "layer bounds must be numeric");
                if args.len() == 4 { anyhow::ensure!(args[3].ty()? == Ty::Bool, "layer condition must be boolean"); }
                return Ok(Ty::Number);
            }
            Self::FirstHeight | Self::LastHeight => {
                anyhow::ensure!((2..=3).contains(&args.len()), "height crossing expects value, threshold, and optional condition");
                anyhow::ensure!(args[..2].iter().all(|arg| arg.ty().ok() == Some(Ty::Number)), "height crossing arguments must be numeric");
                if args.len() == 3 { anyhow::ensure!(args[2].ty()? == Ty::Bool, "height condition must be boolean"); }
                return Ok(Ty::Number);
            }
            Self::CountGates => {
                anyhow::ensure!(args.len() == 1 && args[0].ty()? == Ty::Bool, "count_gates expects one condition");
                return Ok(Ty::Number);
            }
        };
        anyhow::ensure!(args.len() == expected, "function expects {expected} arguments");
        if matches!(self, Self::If) {
            anyhow::ensure!(args[0].ty()? == Ty::Bool, "if condition must be boolean");
            anyhow::ensure!(args[1].ty()? == Ty::Number && args[2].ty()? == Ty::Number, "if branches must be numbers");
        } else {
            anyhow::ensure!(args.iter().all(|arg| arg.ty().ok() == Some(Ty::Number)), "function arguments must be numbers");
        }
        Ok(Ty::Number)
    }

    fn eval(self, args: &[Expr], gate: &GateValues) -> Option<Value> {
        if matches!(self, Self::If) {
            let Value::Bool(condition) = args[0].eval(gate)? else { return None };
            return args[if condition { 1 } else { 2 }].eval(gate);
        }
        let number = |at: usize| match args[at].eval(gate)? {
            Value::Number(value) => Some(value),
            Value::Bool(_) => None,
        };
        let a = number(0)?;
        let b = number(1)?;
        Some(Value::Number(match self {
            Self::Min => a.min(b),
            Self::Max => a.max(b),
            Self::Mean => (a + b) / 2.0,
            Self::Clamp => a.clamp(b, number(2)?),
            Self::Threshold => if a >= b { 1.0 } else { 0.0 },
            Self::If => unreachable!(),
            _ => return None,
        }))
    }

    fn eval_column(self, args: &[Expr], gates: &[GateValues]) -> Option<f32> {
        let condition_at = |at: usize, gate: &GateValues| -> bool {
            at >= args.len() || matches!(args[at].eval(gate), Some(Value::Bool(true)))
        };
        if matches!(self, Self::CountGates) {
            return Some(
                gates
                    .iter()
                    .filter(|gate| matches!(args[0].eval(gate), Some(Value::Bool(true))))
                    .count() as f32,
            );
        }
        let (condition, layer) = match self {
            Self::MaxVertical | Self::MinVertical | Self::MeanVertical => (1, None),
            Self::MaxLayer | Self::MinLayer | Self::MeanLayer => {
                let first = gates.first()?;
                let Value::Number(low) = args[1].eval(first)? else { return None };
                let Value::Number(high) = args[2].eval(first)? else { return None };
                (3, Some((low.min(high), low.max(high))))
            }
            Self::FirstHeight | Self::LastHeight => {
                let first = gates.first()?;
                let Value::Number(threshold) = args[1].eval(first)? else { return None };
                let iter: Box<dyn Iterator<Item = &GateValues>> = if matches!(self, Self::FirstHeight) {
                    Box::new(gates.iter())
                } else {
                    Box::new(gates.iter().rev())
                };
                return iter
                    .filter(|gate| condition_at(2, gate))
                    .find_map(|gate| match args[0].eval(gate) {
                        Some(Value::Number(value)) if value >= threshold => Some(gate.altitude_km),
                        _ => None,
                    });
            }
            _ => return None,
        };
        let values: Vec<f32> = gates
            .iter()
            .filter(|gate| {
                layer.is_none_or(|(low, high)| (low..=high).contains(&gate.altitude_km))
                    && condition_at(condition, gate)
            })
            .filter_map(|gate| match args[0].eval(gate) {
                Some(Value::Number(value)) if value.is_finite() => Some(value),
                _ => None,
            })
            .collect();
        if values.is_empty() { return None; }
        Some(match self {
            Self::MaxVertical | Self::MaxLayer => values.into_iter().fold(f32::NEG_INFINITY, f32::max),
            Self::MinVertical | Self::MinLayer => values.into_iter().fold(f32::INFINITY, f32::min),
            Self::MeanVertical | Self::MeanLayer => values.iter().sum::<f32>() / values.len() as f32,
            _ => unreachable!(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f32),
    Name(String),
    Op(&'static str),
    Left,
    Right,
    Comma,
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    nodes: usize,
}

impl Parser {
    fn new(source: &str) -> anyhow::Result<Self> {
        Ok(Self { tokens: lex(source)?, at: 0, nodes: 0 })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn take(&mut self) -> Option<Token> {
        let token = self.peek()?.clone();
        self.at += 1;
        Some(token)
    }

    fn node(&mut self, expr: Expr, depth: usize) -> anyhow::Result<Expr> {
        self.nodes += 1;
        anyhow::ensure!(self.nodes <= MAX_NODES, "expression has too many nodes");
        anyhow::ensure!(depth <= MAX_DEPTH, "expression is too deeply nested");
        Ok(expr)
    }

    fn expression(&mut self, min_bp: u8, depth: usize) -> anyhow::Result<Expr> {
        let mut left = match self.take() {
            Some(Token::Number(value)) => self.node(Expr::Number(value), depth)?,
            Some(Token::Op("+")) => {
                let value = self.expression(11, depth + 1)?;
                self.node(Expr::Unary(false, Box::new(value)), depth)?
            }
            Some(Token::Op("-")) => {
                let value = self.expression(11, depth + 1)?;
                self.node(Expr::Unary(true, Box::new(value)), depth)?
            }
            Some(Token::Name(name)) if matches!(self.peek(), Some(Token::Left)) => {
                self.take();
                let mut args = Vec::new();
                if !matches!(self.peek(), Some(Token::Right)) {
                    loop {
                        args.push(self.expression(0, depth + 1)?);
                        if !matches!(self.peek(), Some(Token::Comma)) { break; }
                        self.take();
                    }
                }
                anyhow::ensure!(matches!(self.take(), Some(Token::Right)), "missing ')'");
                self.node(Expr::Call(function(&name)?, args), depth)?
            }
            Some(Token::Name(name)) => self.node(Expr::Var(variable(&name)?), depth)?,
            Some(Token::Left) => {
                let expr = self.expression(0, depth + 1)?;
                anyhow::ensure!(matches!(self.take(), Some(Token::Right)), "missing ')'");
                expr
            }
            _ => anyhow::bail!("expected expression"),
        };
        while let Some(Token::Op(symbol)) = self.peek() {
            let (left_bp, right_bp, op) = infix(symbol)?;
            if left_bp < min_bp { break; }
            self.take();
            let right = self.expression(right_bp, depth + 1)?;
            left = self.node(Expr::Binary(op, Box::new(left), Box::new(right)), depth)?;
        }
        Ok(left)
    }
}

fn variable(name: &str) -> anyhow::Result<Var> {
    let upper = name.to_ascii_uppercase();
    if let Some(moment) = Moment::from_code(&upper) { return Ok(Var::Moment(moment)); }
    Ok(match upper.as_str() {
        "ALTITUDE" => Var::Altitude,
        "RANGE" => Var::Range,
        "AZIMUTH" => Var::Azimuth,
        "ELEVATION" => Var::Elevation,
        "FREEZING_LEVEL" => Var::Freezing,
        "MINUS10C_HEIGHT" => Var::Minus10,
        "MINUS20C_HEIGHT" => Var::Minus20,
        _ => anyhow::bail!("unknown input '{name}'"),
    })
}

fn function(name: &str) -> anyhow::Result<Fn> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "min" => Fn::Min,
        "max" => Fn::Max,
        "mean" => Fn::Mean,
        "clamp" => Fn::Clamp,
        "if" => Fn::If,
        "threshold" => Fn::Threshold,
        "max_vertical" => Fn::MaxVertical,
        "min_vertical" => Fn::MinVertical,
        "mean_vertical" => Fn::MeanVertical,
        "max_layer" => Fn::MaxLayer,
        "min_layer" => Fn::MinLayer,
        "mean_layer" => Fn::MeanLayer,
        "first_height" => Fn::FirstHeight,
        "last_height" => Fn::LastHeight,
        "count_gates" => Fn::CountGates,
        _ => anyhow::bail!("unknown function '{name}'"),
    })
}

fn infix(symbol: &str) -> anyhow::Result<(u8, u8, Op)> {
    Ok(match symbol {
        "==" => (1, 2, Op::Eq), "!=" => (1, 2, Op::Ne),
        "<" => (3, 4, Op::Lt), "<=" => (3, 4, Op::Le), ">" => (3, 4, Op::Gt), ">=" => (3, 4, Op::Ge),
        "+" => (5, 6, Op::Add), "-" => (5, 6, Op::Sub),
        "*" => (7, 8, Op::Mul), "/" => (7, 8, Op::Div),
        _ => anyhow::bail!("unknown operator '{symbol}'"),
    })
}

fn lex(source: &str) -> anyhow::Result<Vec<Token>> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() { at += 1; continue; }
        if bytes[at].is_ascii_digit() || bytes[at] == b'.' {
            let start = at;
            while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') { at += 1; }
            if at < bytes.len() && matches!(bytes[at], b'e' | b'E') {
                at += 1;
                if at < bytes.len() && matches!(bytes[at], b'+' | b'-') { at += 1; }
                while at < bytes.len() && bytes[at].is_ascii_digit() { at += 1; }
            }
            out.push(Token::Number(source[start..at].parse()?));
            continue;
        }
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let start = at;
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') { at += 1; }
            out.push(Token::Name(source[start..at].to_string()));
            continue;
        }
        let token = match bytes[at] {
            b'(' => Token::Left, b')' => Token::Right, b',' => Token::Comma,
            b'+' => Token::Op("+"), b'-' => Token::Op("-"), b'*' => Token::Op("*"), b'/' => Token::Op("/"),
            b'<' if bytes.get(at + 1) == Some(&b'=') => { at += 1; Token::Op("<=") },
            b'>' if bytes.get(at + 1) == Some(&b'=') => { at += 1; Token::Op(">=") },
            b'=' if bytes.get(at + 1) == Some(&b'=') => { at += 1; Token::Op("==") },
            b'!' if bytes.get(at + 1) == Some(&b'=') => { at += 1; Token::Op("!=") },
            b'<' => Token::Op("<"), b'>' => Token::Op(">"),
            other => anyhow::bail!("unexpected character '{}'", other as char),
        };
        out.push(token);
        at += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(expression: &str) -> ProductDefinition {
        ProductDefinition {
            version: 1, name: "test".into(), description: "fixture".into(), units: "score".into(), inputs: vec![Moment::Reflectivity, Moment::CorrelationCoefficient],
            expression: expression.into(), palette: "test".into(), min: 0.0, max: 100.0, missing: -999.0,
            environment: Vec::new(),
        }
    }

    #[test]
    fn typed_expression_samples_declared_moments() {
        let product = Product::compile(definition("if(REF >= 40, clamp(REF * CC, 0, 100), 0)")).unwrap();
        let mut gate = GateValues::default();
        gate.moments[Moment::Reflectivity.index()] = Some(50.0);
        gate.moments[Moment::CorrelationCoefficient.index()] = Some(0.9);
        assert_eq!(product.sample(&gate), 45.0);
    }

    #[test]
    fn invalid_and_unbounded_expressions_are_rejected() {
        assert!(Product::compile(definition("REF + VEL")).is_err());
        assert!(Product::compile(definition("if(REF, 1, 0)")).is_err());
        let deep = format!("{}REF{}", "(".repeat(20), ")".repeat(20));
        assert!(Product::compile(definition(&deep)).is_err());
        assert!(Product::compile(definition("REF / 0")).is_ok());
    }

    #[test]
    fn missing_values_and_division_by_zero_use_product_missing() {
        let gate = GateValues::default();
        assert_eq!(Product::compile(definition("REF")).unwrap().sample(&gate), -999.0);
        assert_eq!(Product::compile(definition("1 / 0")).unwrap().sample(&gate), -999.0);
    }

    #[test]
    fn portable_definition_round_trips_and_declares_environment() {
        let mut definition = definition("REF + freezing_level");
        assert!(Product::compile(definition.clone()).is_err());
        definition.environment.push(EnvironmentInput::FreezingLevel);
        let json = serde_json::to_string(&definition).unwrap();
        let restored: ProductDefinition = serde_json::from_str(&json).unwrap();
        assert!(Product::compile(restored).is_ok());
    }

    #[test]
    fn vertical_and_layer_reducers_respect_masks_and_heights() {
        let gates: Vec<_> = [(1.0, 30.0), (3.0, 45.0), (5.0, 55.0)]
            .into_iter()
            .map(|(altitude_km, reflectivity)| {
                let mut gate = GateValues { altitude_km, ..Default::default() };
                gate.moments[Moment::Reflectivity.index()] = Some(reflectivity);
                gate
            })
            .collect();
        assert_eq!(Product::compile(definition("max_vertical(REF, REF >= 40)")).unwrap().column(&gates), 55.0);
        assert_eq!(Product::compile(definition("mean_layer(REF, 2, 4)")).unwrap().column(&gates), 45.0);
        assert_eq!(Product::compile(definition("first_height(REF, 40)")).unwrap().column(&gates), 3.0);
        assert_eq!(Product::compile(definition("last_height(REF, 40)")).unwrap().column(&gates), 5.0);
        assert_eq!(Product::compile(definition("count_gates(REF >= 40)")).unwrap().column(&gates), 2.0);
    }
}
