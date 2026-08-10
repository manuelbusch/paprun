//! JavaScript-Anbindung für WebAssembly.
//!
//! Nur für das Ziel `wasm32` übersetzt; auf allen anderen Zielen ist dieses
//! Modul leer und die Abhängigkeit `wasm-bindgen` entfällt vollständig.
//!
//! ```js
//! import init, { Pap } from "./pkg/paprun.js";
//! await init();
//! const pap = new Pap(await (await fetch("Lohnsteuer2025.xml")).text());
//! const ergebnis = JSON.parse(pap.run(["LZZ", "STKL", "RE4"], ["1", "1", "5000000"]));
//! console.log(ergebnis.LSTLZZ);   // "692700" — Cent
//! ```

use crate::json;
use wasm_bindgen::prelude::*;

/// Ein geladener Programmablaufplan.
#[wasm_bindgen]
pub struct Pap {
    inner: crate::Pap,
}

#[wasm_bindgen]
impl Pap {
    /// Lädt einen PAP aus XML-Pseudocode oder YAML; das Format wird am Inhalt
    /// erkannt.
    #[wasm_bindgen(constructor)]
    pub fn new(source: &str) -> Result<Pap, JsError> {
        crate::Pap::from_source(source)
            .map(|inner| Pap { inner })
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Name des Plans, etwa `Lohnsteuer2025`.
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Eingabevariablen als JSON-Array: `[{name, type, default}, …]`.
    #[wasm_bindgen(js_name = inputVariables)]
    pub fn input_variables(&self) -> String {
        self.inner.input_variables_json()
    }

    /// Ausgabevariablen als JSON-Array: `[{name, type, group}, …]`.
    #[wasm_bindgen(js_name = outputVariables)]
    pub fn output_variables(&self) -> String {
        self.inner.output_variables_json()
    }

    /// Rechnet einen Fall. `names` und `values` sind gleich lange Listen;
    /// nicht genannte Eingaben behalten ihren Default. Das Ergebnis ist ein
    /// JSON-Objekt mit den Ausgabevariablen — Werte als Zeichenketten, damit
    /// keine Nachkommastelle über JavaScripts `Number` verloren geht.
    pub fn run(&self, names: Vec<String>, values: Vec<String>) -> Result<String, JsError> {
        if names.len() != values.len() {
            return Err(JsError::new(
                "Namen und Werte müssen gleich viele Einträge haben",
            ));
        }
        let mut inputs = self.inner.new_inputs();
        for (name, value) in names.iter().zip(&values) {
            inputs
                .set(name, value)
                .map_err(|e| JsError::new(&e.to_string()))?;
        }
        let outputs = self
            .inner
            .run(&inputs)
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(json::object(&outputs))
    }

    /// Wie [`Pap::run`], liefert aber auch alle Zwischenergebnisse.
    #[wasm_bindgen(js_name = runAll)]
    pub fn run_all(&self, names: Vec<String>, values: Vec<String>) -> Result<String, JsError> {
        if names.len() != values.len() {
            return Err(JsError::new(
                "Namen und Werte müssen gleich viele Einträge haben",
            ));
        }
        let mut inputs = self.inner.new_inputs();
        for (name, value) in names.iter().zip(&values) {
            inputs
                .set(name, value)
                .map_err(|e| JsError::new(&e.to_string()))?;
        }
        let outputs = self
            .inner
            .run_all(&inputs)
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(json::object(&outputs))
    }

    /// Der Plan in der lesbaren YAML-Fassung.
    #[wasm_bindgen(js_name = toYaml)]
    pub fn to_yaml(&self) -> String {
        crate::emit::to_yaml(&self.inner)
    }
}
