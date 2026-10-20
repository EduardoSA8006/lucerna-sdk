//! Saída legível padrão da linha de comando.

use serde_json::Value;

/// Texto legível de um resultado.
///
/// - string → ela mesma;
/// - objeto → `chave: valor` por linha, em ordem de chave;
/// - lista → um item por linha;
/// - `null` → nada.
///
/// Valores aninhados saem em JSON compacto; strings dentro de objeto ou lista
/// saem sem aspas. Sem `\n` no fim.
pub fn render_default(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            keys.iter()
                .map(|k| format!("{k}: {}", inline(&map[k.as_str()])))
                .collect::<Vec<_>>()
                .join("\n")
        }
        Value::Array(items) => items.iter().map(inline).collect::<Vec<_>>().join("\n"),
        other => inline(other),
    }
}

/// Um valor numa linha: string sem aspas, o resto em JSON compacto.
fn inline(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
