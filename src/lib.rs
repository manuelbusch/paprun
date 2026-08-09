//! Interpreter für die XML-Pseudocodes der BMF-Programmablaufpläne (Lohnsteuer).
//!
//! ```no_run
//! # fn main() -> Result<(), paprun::Error> {
//! let xml = std::fs::read_to_string("tests/data/Lohnsteuer2025.xml").unwrap();
//! let pap = paprun::Pap::from_xml(&xml)?;
//! let mut inputs = pap.new_inputs();
//! inputs.set("RE4", "5000000")?;
//! inputs.set("STKL", "1")?;
//! for (name, value) in pap.run(&inputs)? {
//!     println!("{name}={value}");
//! }
//! # Ok(()) }
//! ```

pub mod ast;
pub mod batch;
pub mod emit;
pub mod error;
pub mod eval;
pub mod lex;
pub mod load;
pub mod load_yaml;
pub mod parse;
pub mod value;

use ast::{Ty, VarId, VarKind};
use bigdecimal::BigDecimal;

pub use ast::Pap;
pub use error::Error;
pub use value::{RoundMode, Value};

/// Gesetzte Eingabewerte für einen Lauf.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    values: Vec<(VarId, Value)>,
}

impl Pap {
    /// Lädt einen PAP aus dem XML-Pseudocode.
    pub fn from_xml(xml: &str) -> Result<Pap, Error> {
        load::load(xml)
    }

    /// Lädt einen PAP aus dem YAML-Format.
    pub fn from_yaml(yaml: &str) -> Result<Pap, Error> {
        load_yaml::load(yaml)
    }

    /// Lädt einen PAP und erkennt das Format am Inhalt: Beginnt die Datei
    /// (nach Kommentaren und Leerzeilen) mit `<`, wird XML angenommen.
    pub fn from_source(source: &str) -> Result<Pap, Error> {
        let looks_like_xml = source
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with('<');
        if looks_like_xml {
            Pap::from_xml(source)
        } else {
            Pap::from_yaml(source)
        }
    }

    pub fn new_inputs(&self) -> InputBuilder<'_> {
        InputBuilder {
            pap: self,
            inputs: Inputs::default(),
        }
    }

    /// Führt `MAIN` aus und liefert die Ausgabevariablen in Deklarationsreihenfolge.
    pub fn run(&self, inputs: &InputBuilder<'_>) -> Result<Vec<(String, String)>, Error> {
        let env = eval::run(self, &inputs.inputs.values)?;
        Ok(self
            .vars_of_kind(VarKind::Output)
            .map(|(id, decl)| (decl.name.clone(), format_value(&env.slots[id as usize])))
            .collect())
    }

    /// Wie [`Pap::run`], liefert aber den Endzustand *aller* Variablen —
    /// nützlich zum Nachvollziehen von Zwischenergebnissen.
    pub fn run_all(&self, inputs: &InputBuilder<'_>) -> Result<Vec<(String, String)>, Error> {
        let env = eval::run(self, &inputs.inputs.values)?;
        Ok(self
            .vars
            .iter()
            .zip(&env.slots)
            .map(|(decl, value)| (decl.name.clone(), format_value(value)))
            .collect())
    }
}

/// Sammelt Eingabewerte und prüft sie gegen die deklarierten Typen.
#[derive(Debug, Clone)]
pub struct InputBuilder<'a> {
    pap: &'a Pap,
    inputs: Inputs,
}

impl InputBuilder<'_> {
    /// Setzt eine Eingabevariable aus ihrer Textdarstellung.
    pub fn set(&mut self, name: &str, text: &str) -> Result<&mut Self, Error> {
        let id = self
            .pap
            .var_id(name)
            .ok_or_else(|| Error::eval(format!("unbekannte Variable `{name}`")))?;
        let decl = self.pap.var(id);
        if decl.kind != VarKind::Input {
            return Err(Error::eval(format!(
                "`{name}` ist keine Eingabevariable, sondern {:?}",
                decl.kind
            )));
        }
        let value = parse_input_value(text, decl.ty, name)?;
        self.inputs.values.retain(|(existing, _)| *existing != id);
        self.inputs.values.push((id, value));
        Ok(self)
    }

    /// Wie [`InputBuilder::set`], aber über eine bereits aufgelöste `VarId`.
    /// Für Stapelverarbeitung, wo die Spalten einmal pro Datei aufgelöst
    /// werden statt einmal pro Zeile.
    pub fn set_by_id(&mut self, id: VarId, text: &str) -> Result<&mut Self, Error> {
        let decl = self.pap.var(id);
        if decl.kind != VarKind::Input {
            return Err(Error::eval(format!(
                "`{}` ist keine Eingabevariable, sondern {:?}",
                decl.name, decl.kind
            )));
        }
        let value = parse_input_value(text, decl.ty, &decl.name)?;
        self.inputs.values.retain(|(existing, _)| *existing != id);
        self.inputs.values.push((id, value));
        Ok(self)
    }
}

fn parse_input_value(text: &str, ty: Ty, name: &str) -> Result<Value, Error> {
    let text = text.trim();
    let invalid = |expected: &str| {
        Error::eval(format!(
            "`{name}`: `{text}` ist kein gültiger Wert für {expected}"
        ))
    };
    Ok(match ty {
        Ty::Int => Value::Int(text.parse().map_err(|_| invalid("int"))?),
        Ty::Dec => Value::Dec(
            text.parse::<BigDecimal>()
                .map_err(|_| invalid("BigDecimal"))?,
        ),
        Ty::Dbl => Value::Dbl(text.parse().map_err(|_| invalid("double"))?),
        Ty::DecArr => return Err(Error::eval(format!("`{name}`: Arrays sind keine Eingaben"))),
    })
}

/// Ausgabeformatierung: BigDecimal stets in Plain-Notation (nie wissenschaftlich).
pub fn format_value(value: &Value) -> String {
    match value {
        Value::Int(i) => i.to_string(),
        Value::Dec(d) => value::to_plain_string(d),
        Value::Dbl(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Arr(items) => {
            let inner: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", inner.join(", "))
        }
    }
}
