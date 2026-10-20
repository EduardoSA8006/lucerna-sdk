//! Nomes de métodos reservados.

/// Aperto de mão inicial.
pub const HELLO: &str = "lucerna.hello";
/// Assina tópicos.
pub const SUBSCRIBE: &str = "lucerna.subscribe";
/// Cancela assinaturas.
pub const UNSUBSCRIBE: &str = "lucerna.unsubscribe";
/// Notificação com o retrato de um tópico.
pub const STATE: &str = "lucerna.state";
/// Põe ou atualiza um item na barra do shell.
pub const BAR_SET: &str = "bar.set";
/// Tira um item da barra do shell.
pub const BAR_CLEAR: &str = "bar.clear";
/// Pede o não perturbe.
pub const DND_REQUEST: &str = "dnd.request";
/// Devolve o não perturbe.
pub const DND_RELEASE: &str = "dnd.release";
/// Pede ao shell para abrir a TUI do app.
pub const TUI_OPEN: &str = "tui.open";
/// Pede ao shell para abrir o painel do app.
pub const PANEL_OPEN: &str = "panel.open";

/// Métodos que começam com `lucerna.` são reservados ao protocolo.
pub fn is_reserved(method: &str) -> bool {
    method.starts_with("lucerna.")
}
