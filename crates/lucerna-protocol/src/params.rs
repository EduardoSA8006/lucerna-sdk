//! Parâmetros e resultados dos métodos do protocolo.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Version;

/// Parâmetros do `lucerna.hello`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HelloParams {
    /// Versão do protocolo do cliente.
    pub protocol: Version,
    /// Id de quem chama.
    pub client: String,
    /// Id do app procurado.
    pub app: String,
}

/// Resultado do `lucerna.hello`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    /// Versão do protocolo do serviço.
    pub protocol: Version,
    /// Id do app.
    pub app: String,
    /// Versão do app.
    pub version: String,
    /// Tópicos comuns.
    pub topics: Vec<String>,
    /// Tópicos contínuos.
    pub streams: Vec<String>,
    /// Métodos do app.
    pub methods: Vec<String>,
}

/// Parâmetros do `lucerna.subscribe` e do `lucerna.unsubscribe`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicsParams {
    /// Tópicos.
    pub topics: Vec<String>,
}

/// Retrato de um tópico (params do `lucerna.state`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Tópico.
    pub topic: String,
    /// Número de sequência, por tópico.
    pub seq: u64,
    /// Retrato inteiro do tópico.
    pub data: Value,
}

/// Resultado do `lucerna.subscribe`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscribeResult {
    /// Retratos atuais dos tópicos assinados.
    pub states: Vec<State>,
}

/// Item da barra do shell (`bar.set`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarItem {
    /// Id do item.
    pub id: String,
    /// Ícone.
    pub icon: String,
    /// Texto opcional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Fim, em segundos Unix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<u64>,
    /// Abre o painel ao clicar.
    #[serde(default)]
    pub panel: bool,
}

/// Parâmetros do `bar.clear`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarClearParams {
    /// Id do item.
    pub id: String,
}

/// Parâmetros do `dnd.request`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DndRequestParams {
    /// Motivo do pedido.
    pub reason: String,
}

/// Parâmetros do `tui.open`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TuiOpenParams {
    /// Argumentos da TUI.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Parâmetros do `panel.open`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct PanelOpenParams {
    /// Página do painel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}
