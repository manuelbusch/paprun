//! Auswertung von Ausdrücken und Anweisungen.

use crate::ast::{BdMethod, BinOp, Expr, MethodId, Pap, Stmt, Ty, VarKind};
use crate::error::Error;
use crate::value::{RoundMode, Value, div_exact, div_scale, long_value, set_scale, value_of_f64};
use bigdecimal::BigDecimal;
use num_traits::Zero;

/// Variablenzustand eines Laufs, indiziert über `VarId`.
#[derive(Debug, Clone)]
pub struct Env {
    pub slots: Vec<Value>,
}

/// Wertet einen Ausdruck ohne Variablenzugriff aus (Defaults, Konstanten).
pub fn const_eval(expr: &Expr) -> Result<Value, Error> {
    eval_expr(expr, &[])
}

pub fn eval_expr(expr: &Expr, slots: &[Value]) -> Result<Value, Error> {
    Ok(match expr {
        Expr::IntLit(i) => Value::Int(*i),
        Expr::DecLit(d) => Value::Dec(d.clone()),
        Expr::DblLit(f) => Value::Dbl(*f),
        Expr::RoundLit(_) => {
            return Err(Error::eval("Rundungsmodus ist kein auswertbarer Wert"));
        }
        Expr::Var(id) => slots
            .get(*id as usize)
            .cloned()
            .ok_or_else(|| Error::eval(format!("Variablenslot {id} existiert nicht")))?,
        Expr::Index { arr, idx } => {
            let arr = eval_expr(arr, slots)?;
            let i = as_int(&eval_expr(idx, slots)?, "Array-Index")?;
            let Value::Arr(items) = arr else {
                return Err(Error::eval(format!(
                    "Indexzugriff auf Nicht-Array ({})",
                    arr.type_name()
                )));
            };
            let i = usize::try_from(i)
                .ok()
                .filter(|i| *i < items.len())
                .ok_or_else(|| {
                    Error::eval(format!("Array-Index {i} außerhalb 0..{}", items.len()))
                })?;
            items[i].clone()
        }
        Expr::Call { recv, method, args } => {
            let recv = eval_expr(recv, slots)?;
            eval_call(&recv, *method, args, slots)?
        }
        Expr::ValueOf(arg) => match eval_expr(arg, slots)? {
            Value::Int(i) => Value::Dec(BigDecimal::from(i)),
            Value::Dbl(f) => Value::Dec(value_of_f64(f)?),
            other => {
                return Err(Error::eval(format!(
                    "BigDecimal.valueOf erwartet int oder double, erhielt {}",
                    other.type_name()
                )));
            }
        },
        Expr::NewBigDecimal(arg) => {
            match eval_expr(arg, slots)? {
                Value::Int(i) => Value::Dec(BigDecimal::from(i)),
                // Java `new BigDecimal(double)` übernimmt den Binärwert exakt.
                Value::Dbl(f) => Value::Dec(BigDecimal::try_from(f).map_err(|e| {
                    Error::eval(format!("new BigDecimal({f}) nicht darstellbar: {e}"))
                })?),
                other => {
                    return Err(Error::eval(format!(
                        "new BigDecimal erwartet int oder double, erhielt {}",
                        other.type_name()
                    )));
                }
            }
        }
        Expr::Neg(inner) => match eval_expr(inner, slots)? {
            Value::Int(i) => Value::Int(-i),
            Value::Dbl(f) => Value::Dbl(-f),
            Value::Dec(d) => Value::Dec(-d),
            other => {
                return Err(Error::eval(format!(
                    "unäres Minus auf {}",
                    other.type_name()
                )));
            }
        },
        Expr::Not(inner) => Value::Bool(!as_bool(&eval_expr(inner, slots)?)?),
        Expr::Bin { op, lhs, rhs } => eval_bin(*op, lhs, rhs, slots)?,
    })
}

fn eval_bin(op: BinOp, lhs: &Expr, rhs: &Expr, slots: &[Value]) -> Result<Value, Error> {
    // Kurzschlussauswertung wie in Java.
    if matches!(op, BinOp::And | BinOp::Or) {
        let l = as_bool(&eval_expr(lhs, slots)?)?;
        return Ok(Value::Bool(match op {
            BinOp::And if !l => false,
            BinOp::Or if l => true,
            _ => as_bool(&eval_expr(rhs, slots)?)?,
        }));
    }

    let l = eval_expr(lhs, slots)?;
    let r = eval_expr(rhs, slots)?;

    if matches!(op, BinOp::Eq | BinOp::Ne)
        && let (Value::Bool(a), Value::Bool(b)) = (&l, &r)
    {
        return Ok(Value::Bool(if op == BinOp::Eq { a == b } else { a != b }));
    }

    // Java-Promotion: sobald ein Operand `double` ist, wird in `double` gerechnet.
    let both_int = matches!((&l, &r), (Value::Int(_), Value::Int(_)));
    if both_int {
        let (a, b) = (as_int(&l, "Operand")?, as_int(&r, "Operand")?);
        return Ok(match op {
            BinOp::Add => Value::Int(a + b),
            BinOp::Sub => Value::Int(a - b),
            BinOp::Mul => Value::Int(a * b),
            BinOp::Div => {
                if b == 0 {
                    return Err(Error::eval("ganzzahlige Division durch null"));
                }
                Value::Int(a / b)
            }
            BinOp::Rem => {
                if b == 0 {
                    return Err(Error::eval("Modulo durch null"));
                }
                Value::Int(a % b)
            }
            BinOp::Eq => Value::Bool(a == b),
            BinOp::Ne => Value::Bool(a != b),
            BinOp::Lt => Value::Bool(a < b),
            BinOp::Le => Value::Bool(a <= b),
            BinOp::Gt => Value::Bool(a > b),
            BinOp::Ge => Value::Bool(a >= b),
            BinOp::And | BinOp::Or => unreachable!("oben behandelt"),
        });
    }

    let (a, b) = (as_f64(&l, op)?, as_f64(&r, op)?);
    Ok(match op {
        BinOp::Add => Value::Dbl(a + b),
        BinOp::Sub => Value::Dbl(a - b),
        BinOp::Mul => Value::Dbl(a * b),
        BinOp::Div => Value::Dbl(a / b),
        BinOp::Rem => Value::Dbl(a % b),
        BinOp::Eq => Value::Bool(a == b),
        BinOp::Ne => Value::Bool(a != b),
        BinOp::Lt => Value::Bool(a < b),
        BinOp::Le => Value::Bool(a <= b),
        BinOp::Gt => Value::Bool(a > b),
        BinOp::Ge => Value::Bool(a >= b),
        BinOp::And | BinOp::Or => unreachable!("oben behandelt"),
    })
}

fn eval_call(
    recv: &Value,
    method: BdMethod,
    args: &[Expr],
    slots: &[Value],
) -> Result<Value, Error> {
    let Value::Dec(recv) = recv else {
        return Err(Error::eval(format!(
            "Methodenaufruf auf {} — nur BigDecimal wird unterstützt",
            recv.type_name()
        )));
    };

    // Rundungsmodi sind reine Syntax und werden nicht als Wert ausgewertet.
    let mode_at = |i: usize| -> Result<RoundMode, Error> {
        match args.get(i) {
            Some(Expr::RoundLit(m)) => Ok(*m),
            other => Err(Error::eval(format!(
                "Rundungsmodus als Argument {i} erwartet, erhielt {other:?}"
            ))),
        }
    };
    let dec_at = |i: usize| -> Result<BigDecimal, Error> {
        as_dec(&eval_expr(&args[i], slots)?, "Argument")
    };
    let int_at =
        |i: usize| -> Result<i64, Error> { as_int(&eval_expr(&args[i], slots)?, "Argument") };

    let arity_err = |expected: &str| {
        Error::eval(format!(
            "{method:?}: {expected} Argument(e) erwartet, erhielt {}",
            args.len()
        ))
    };

    Ok(match (method, args.len()) {
        (BdMethod::Add, 1) => Value::Dec(recv + dec_at(0)?),
        (BdMethod::Subtract, 1) => Value::Dec(recv - dec_at(0)?),
        (BdMethod::Multiply, 1) => Value::Dec(recv * dec_at(0)?),
        (BdMethod::Divide, 1) => Value::Dec(div_exact(recv, &dec_at(0)?)?),
        // Java divide(divisor, roundingMode) rundet auf den Scale des Empfängers.
        (BdMethod::Divide, 2) => {
            let scale = recv.as_bigint_and_exponent().1;
            Value::Dec(div_scale(recv, &dec_at(0)?, scale, mode_at(1)?)?)
        }
        (BdMethod::Divide, 3) => Value::Dec(div_scale(recv, &dec_at(0)?, int_at(1)?, mode_at(2)?)?),
        (BdMethod::SetScale, 2) => Value::Dec(set_scale(recv, int_at(0)?, mode_at(1)?)?),
        (BdMethod::SetScale, 1) => {
            // Java wirft hier, wenn gerundet werden müsste.
            let scale = int_at(0)?;
            let rounded = set_scale(recv, scale, RoundMode::Down)?;
            if &rounded - recv != BigDecimal::zero() {
                return Err(Error::eval(format!(
                    "setScale({scale}) ohne Rundungsmodus würde {recv} runden"
                )));
            }
            Value::Dec(rounded)
        }
        (BdMethod::CompareTo, 1) => {
            let other = dec_at(0)?;
            Value::Int(match recv.cmp(&other) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            })
        }
        (BdMethod::LongValue, 0) => Value::Int(long_value(recv)?),
        (BdMethod::IntValue, 0) => Value::Int(long_value(recv)? as i32 as i64),
        (BdMethod::Negate, 0) => Value::Dec(-recv.clone()),
        (BdMethod::Abs, 0) => Value::Dec(recv.abs()),
        (BdMethod::Add | BdMethod::Subtract | BdMethod::Multiply, _) => {
            return Err(arity_err("genau 1"));
        }
        (BdMethod::Divide, _) => return Err(arity_err("1, 2 oder 3")),
        (BdMethod::SetScale, _) => return Err(arity_err("1 oder 2")),
        (BdMethod::CompareTo, _) => return Err(arity_err("genau 1")),
        (BdMethod::LongValue | BdMethod::IntValue | BdMethod::Negate | BdMethod::Abs, _) => {
            return Err(arity_err("keine"));
        }
    })
}

fn as_bool(v: &Value) -> Result<bool, Error> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(Error::eval(format!(
            "Wahrheitswert erwartet, erhielt {}",
            other.type_name()
        ))),
    }
}

fn as_int(v: &Value, what: &str) -> Result<i64, Error> {
    match v {
        Value::Int(i) => Ok(*i),
        other => Err(Error::eval(format!(
            "{what}: int erwartet, erhielt {}",
            other.type_name()
        ))),
    }
}

fn as_f64(v: &Value, op: BinOp) -> Result<f64, Error> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Dbl(f) => Ok(*f),
        other => Err(Error::eval(format!(
            "Operator {op:?} ist für {} nicht definiert",
            other.type_name()
        ))),
    }
}

fn as_dec(v: &Value, what: &str) -> Result<BigDecimal, Error> {
    match v {
        Value::Dec(d) => Ok(d.clone()),
        other => Err(Error::eval(format!(
            "{what}: BigDecimal erwartet, erhielt {}",
            other.type_name()
        ))),
    }
}

/// Passt einen berechneten Wert an den deklarierten Typ der Zielvariablen an.
/// Erlaubt ist nur Javas `int` → `double`-Erweiterung; alles andere ist ein Fehler,
/// damit semantische Abweichungen sofort auffallen.
pub fn coerce(value: Value, ty: Ty, var_name: &str) -> Result<Value, Error> {
    match (&value, ty) {
        (Value::Int(_), Ty::Int)
        | (Value::Dec(_), Ty::Dec)
        | (Value::Dbl(_), Ty::Dbl)
        | (Value::Arr(_), Ty::DecArr) => Ok(value),
        (Value::Int(i), Ty::Dbl) => Ok(Value::Dbl(*i as f64)),
        _ => Err(Error::eval(format!(
            "Zuweisung an `{var_name}`: {} passt nicht zum deklarierten Typ {}",
            value.type_name(),
            ty.name()
        ))),
    }
}

/// Führt einen Anweisungsblock aus.
fn exec_block(pap: &Pap, env: &mut Env, block: &[Stmt]) -> Result<(), Error> {
    for stmt in block {
        match stmt {
            Stmt::Eval { target, expr } => {
                let value = eval_expr(expr, &env.slots)?;
                let decl = pap.var(*target);
                env.slots[*target as usize] = coerce(value, decl.ty, &decl.name)?;
            }
            Stmt::Execute(id) => exec_method(pap, env, *id)?,
            Stmt::If { cond, then_, else_ } => {
                let taken = if as_bool(&eval_expr(cond, &env.slots)?)? {
                    then_
                } else {
                    else_
                };
                exec_block(pap, env, taken)?;
            }
        }
    }
    Ok(())
}

fn exec_method(pap: &Pap, env: &mut Env, id: MethodId) -> Result<(), Error> {
    let method = &pap.methods[id as usize];
    exec_block(pap, env, &method.body)
        .map_err(|e| Error::eval(format!("in Methode {}: {e}", method.name)))
}

/// Führt `MAIN` aus und liefert den Endzustand aller Variablen.
pub fn run(pap: &Pap, inputs: &[(u32, Value)]) -> Result<Env, Error> {
    let mut env = Env {
        slots: pap.vars.iter().map(|v| v.default.clone()).collect(),
    };
    for (id, value) in inputs {
        let decl = pap.var(*id);
        if decl.kind != VarKind::Input {
            return Err(Error::eval(format!(
                "`{}` ist keine Eingabevariable ({:?})",
                decl.name, decl.kind
            )));
        }
        env.slots[*id as usize] = coerce(value.clone(), decl.ty, &decl.name)?;
    }
    exec_block(pap, &mut env, &pap.main)?;
    Ok(env)
}
