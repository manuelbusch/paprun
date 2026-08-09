use std::fmt;

/// Fehler beim Laden (XML/Parser) oder Auswerten eines PAP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Fehler beim Laden/Parsen; `context` nennt den betroffenen Ausdruck
    /// bzw. das betroffene XML-Element.
    Load { msg: String, context: String },
    /// Fehler zur Auswertungszeit (z. B. Division durch null, Typfehler).
    Eval { msg: String },
}

impl Error {
    pub fn load(msg: impl Into<String>, context: impl Into<String>) -> Self {
        Error::Load {
            msg: msg.into(),
            context: context.into(),
        }
    }

    pub fn eval(msg: impl Into<String>) -> Self {
        Error::Eval { msg: msg.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Load { msg, context } if context.is_empty() => {
                write!(f, "Ladefehler: {msg}")
            }
            Error::Load { msg, context } => {
                write!(f, "Ladefehler: {msg} (in: `{context}`)")
            }
            Error::Eval { msg } => write!(f, "Auswertungsfehler: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
