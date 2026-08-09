//! AST des PAP-Programms.

use crate::value::{RoundMode, Value};
use bigdecimal::BigDecimal;
use std::collections::HashMap;

pub type VarId = u32;
pub type MethodId = u32;

/// Deklarierter Typ einer PAP-Variablen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Int,
    Dec,
    Dbl,
    DecArr,
}

impl Ty {
    pub fn from_xml(s: &str) -> Option<Ty> {
        Some(match s.trim() {
            "int" | "long" => Ty::Int,
            "BigDecimal" => Ty::Dec,
            "double" => Ty::Dbl,
            "BigDecimal[]" => Ty::DecArr,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Ty::Int => "int",
            Ty::Dec => "BigDecimal",
            Ty::Dbl => "double",
            Ty::DecArr => "BigDecimal[]",
        }
    }
}

/// Herkunft einer Variablen im PAP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Input,
    Output,
    Internal,
    Constant,
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: String,
    pub kind: VarKind,
    pub ty: Ty,
    pub default: Value,
    /// Untergruppe der OUTPUTS (`STANDARD`, `DBA`), sonst leer.
    pub group: String,
}

/// Instanzmethoden auf `BigDecimal`, die im PAP vorkommen dürfen.
/// Geschlossenes Enum: Unbekanntes schlägt beim Laden fehl, nicht zur Laufzeit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BdMethod {
    Add,
    Subtract,
    Multiply,
    Divide,
    SetScale,
    CompareTo,
    LongValue,
    IntValue,
    Negate,
    Abs,
}

impl BdMethod {
    pub fn from_name(name: &str) -> Option<BdMethod> {
        Some(match name {
            "add" => BdMethod::Add,
            "subtract" => BdMethod::Subtract,
            "multiply" => BdMethod::Multiply,
            "divide" => BdMethod::Divide,
            "setScale" => BdMethod::SetScale,
            "compareTo" => BdMethod::CompareTo,
            "longValue" => BdMethod::LongValue,
            "intValue" => BdMethod::IntValue,
            "negate" => BdMethod::Negate,
            "abs" => BdMethod::Abs,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(i64),
    DecLit(BigDecimal),
    DblLit(f64),
    RoundLit(RoundMode),
    Var(VarId),
    Index {
        arr: Box<Expr>,
        idx: Box<Expr>,
    },
    Call {
        recv: Box<Expr>,
        method: BdMethod,
        args: Vec<Expr>,
    },
    /// `BigDecimal.valueOf(x)`
    ValueOf(Box<Expr>),
    /// `new BigDecimal(x)`
    NewBigDecimal(Box<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Eval {
        target: VarId,
        expr: Expr,
    },
    Execute(MethodId),
    If {
        cond: Expr,
        then_: Vec<Stmt>,
        else_: Vec<Stmt>,
    },
}

#[derive(Debug, Clone)]
pub struct MethodDef {
    pub name: String,
    pub body: Vec<Stmt>,
}

/// Ein vollständig geladener und aufgelöster Programmablaufplan.
#[derive(Debug, Clone)]
pub struct Pap {
    pub name: String,
    pub version: String,
    pub vars: Vec<VarDecl>,
    pub by_name: HashMap<String, VarId>,
    pub methods: Vec<MethodDef>,
    pub main: Vec<Stmt>,
}

impl Pap {
    pub fn var_id(&self, name: &str) -> Option<VarId> {
        self.by_name.get(name).copied()
    }

    pub fn var(&self, id: VarId) -> &VarDecl {
        &self.vars[id as usize]
    }

    pub fn vars_of_kind(&self, kind: VarKind) -> impl Iterator<Item = (VarId, &VarDecl)> {
        self.vars
            .iter()
            .enumerate()
            .filter(move |(_, v)| v.kind == kind)
            .map(|(i, v)| (i as VarId, v))
    }
}
